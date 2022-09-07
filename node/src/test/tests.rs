use std::io;
use std::sync::Arc;

use crossbeam_channel as chan;
use nakamoto_net as nakamoto;
use nakamoto_net::simulator;
use nakamoto_net::simulator::{Peer as _, Simulation};
use nakamoto_net::Protocol as _;

use crate::collections::{HashMap, HashSet};
use crate::protocol::config::*;
use crate::protocol::message::*;
use crate::protocol::peer::*;
use crate::protocol::*;
use crate::storage::git::Storage;
use crate::storage::ReadStorage;
use crate::test::crypto::MockSigner;
use crate::test::fixtures;
#[allow(unused)]
use crate::test::logger;
use crate::test::peer::Peer;
use crate::test::storage::MockStorage;
use crate::*;

// NOTE
//
// If you wish to see the logs for a running test, simply add the following line to your test:
//
//      logger::init(log::Level::Debug);
//
// You may then run the test with eg. `cargo test -- --nocapture` to always show output.

#[test]
fn test_outbound_connection() {
    let mut alice = Peer::new("alice", [8, 8, 8, 8], MockStorage::empty());
    let bob = Peer::new("bob", [9, 9, 9, 9], MockStorage::empty());
    let eve = Peer::new("eve", [7, 7, 7, 7], MockStorage::empty());

    alice.connect_to(&bob);
    alice.connect_to(&eve);

    let peers = alice
        .protocol
        .peers()
        .negotiated()
        .map(|(ip, _)| *ip)
        .collect::<Vec<_>>();

    assert!(peers.contains(&eve.ip));
    assert!(peers.contains(&bob.ip));
}

#[test]
fn test_inbound_connection() {
    let mut alice = Peer::new("alice", [8, 8, 8, 8], MockStorage::empty());
    let bob = Peer::new("bob", [9, 9, 9, 9], MockStorage::empty());
    let eve = Peer::new("eve", [7, 7, 7, 7], MockStorage::empty());

    alice.connect_from(&bob);
    alice.connect_from(&eve);

    let peers = alice
        .protocol
        .peers()
        .negotiated()
        .map(|(ip, _)| *ip)
        .collect::<Vec<_>>();

    assert!(peers.contains(&eve.ip));
    assert!(peers.contains(&bob.ip));
}

#[test]
fn test_persistent_peer_connect() {
    let rng = fastrand::Rng::new();
    let bob = Peer::new("bob", [8, 8, 8, 8], MockStorage::empty());
    let eve = Peer::new("eve", [9, 9, 9, 9], MockStorage::empty());
    let config = Config {
        connect: vec![bob.addr(), eve.addr()],
        ..Config::default()
    };
    let mut alice = Peer::config(
        "alice",
        config,
        [7, 7, 7, 7],
        vec![],
        MockStorage::empty(),
        rng,
    );

    alice.initialize();

    let mut outbox = alice.outbox();
    assert_matches!(outbox.next(), Some(Io::Connect(a)) if a == bob.addr());
    assert_matches!(outbox.next(), Some(Io::Connect(a)) if a == eve.addr());
    assert_matches!(outbox.next(), None);
}

#[test]
#[ignore]
fn test_wrong_peer_version() {
    // TODO
}

#[test]
fn test_handshake_invalid_timestamp() {
    let mut alice = Peer::new("alice", [7, 7, 7, 7], MockStorage::empty());
    let bob = Peer::new("bob", [8, 8, 8, 8], MockStorage::empty());
    let time_delta = MAX_TIME_DELTA.as_secs() + 1;
    let local = std::net::SocketAddr::new(bob.ip, bob.rng.u16(..));

    alice.initialize();
    alice.connected(bob.addr(), &local, Link::Inbound);
    alice.receive(
        &bob.addr(),
        Message::hello(
            bob.id(),
            alice.timestamp() - time_delta,
            vec![],
            bob.git_url(),
        ),
    );
    assert_matches!(alice.outbox().next(), Some(Io::Disconnect(addr, _)) if addr == bob.addr());
}

#[test]
#[ignore]
fn test_wrong_peer_magic() {
    // TODO
}

#[test]
fn test_inventory_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let mut alice = Peer::new(
        "alice",
        [7, 7, 7, 7],
        Storage::open(tmp.path().join("alice"), MockSigner::default()).unwrap(),
    );
    let bob_storage = fixtures::storage(tmp.path().join("bob"));
    let bob = Peer::new("bob", [8, 8, 8, 8], bob_storage);
    let now = LocalTime::now().as_secs();
    let projs = bob.storage().inventory().unwrap();

    alice.connect_to(&bob);
    alice.receive(
        &bob.addr(),
        Message::Inventory {
            timestamp: now,
            inv: projs.clone(),
            origin: None,
        },
    );

    for proj in &projs {
        let seeds = alice.routing().get(proj).unwrap();
        assert!(seeds.contains(&bob.id()));
    }

    let a = alice
        .storage()
        .inventory()
        .unwrap()
        .into_iter()
        .collect::<HashSet<_>>();
    let b = projs.into_iter().collect::<HashSet<_>>();

    assert_eq!(a, b);
}

#[test]
fn test_tracking() {
    let mut alice = Peer::config(
        "alice",
        Config {
            project_tracking: ProjectTracking::Allowed(HashSet::default()),
            ..Config::default()
        },
        [7, 7, 7, 7],
        vec![],
        MockStorage::empty(),
        fastrand::Rng::new(),
    );
    let proj_id: identity::ProjId = test::arbitrary::gen(1);

    let (sender, receiver) = chan::bounded(1);
    alice.command(Command::Track(proj_id.clone(), sender));
    let policy_change = receiver
        .recv()
        .map_err(client::handle::Error::from)
        .unwrap();
    assert!(policy_change);
    assert!(alice.config().is_tracking(&proj_id));

    let (sender, receiver) = chan::bounded(1);
    alice.command(Command::Untrack(proj_id.clone(), sender));
    let policy_change = receiver
        .recv()
        .map_err(client::handle::Error::from)
        .unwrap();
    assert!(policy_change);
    assert!(!alice.config().is_tracking(&proj_id));
}

#[test]
fn test_inventory_relay_bad_timestamp() {
    let mut alice = Peer::new("alice", [7, 7, 7, 7], MockStorage::empty());
    let bob = Peer::new("bob", [8, 8, 8, 8], MockStorage::empty());
    let two_hours = 3600 * 2;
    let timestamp = alice.local_time.as_secs() - two_hours;

    alice.connect_to(&bob);
    alice.receive(
        &bob.addr(),
        Message::Inventory {
            timestamp,
            inv: vec![],
            origin: None,
        },
    );
    assert_matches!(
        alice.outbox().next(),
        Some(Io::Disconnect(addr, DisconnectReason::Error(PeerError::InvalidTimestamp(t))))
        if addr == bob.addr() && t == timestamp
    );
}

#[test]
fn test_inventory_relay() {
    // Topology is eve <-> alice <-> bob
    let mut alice = Peer::new("alice", [7, 7, 7, 7], MockStorage::empty());
    let bob = Peer::new("bob", [8, 8, 8, 8], MockStorage::empty());
    let eve = Peer::new("eve", [9, 9, 9, 9], MockStorage::empty());
    let inv = vec![];
    let now = LocalTime::now().as_secs();

    // Inventory from Bob relayed to Eve.
    alice.connect_to(&bob);
    alice.connect_from(&eve);
    alice.receive(
        &bob.addr(),
        Message::Inventory {
            timestamp: now,
            inv: inv.clone(),
            origin: None,
        },
    );
    assert_matches!(
        alice.messages(&eve.addr()).next(),
        Some(Message::Inventory { timestamp, origin, .. })
        if origin == Some(bob.id()) && timestamp == now
    );
    assert_matches!(
        alice.messages(&bob.addr()).next(),
        None,
        "The inventory is not sent back to Bob"
    );

    alice.receive(
        &bob.addr(),
        Message::Inventory {
            timestamp: now,
            inv: inv.clone(),
            origin: None,
        },
    );
    assert_matches!(
        alice.messages(&eve.addr()).next(),
        None,
        "Sending the same inventory again doesn't trigger a relay"
    );

    alice.receive(
        &bob.addr(),
        Message::Inventory {
            timestamp: now + 1,
            inv: inv.clone(),
            origin: None,
        },
    );
    assert_matches!(
        alice.messages(&eve.addr()).next(),
        Some(Message::Inventory { timestamp, origin, .. })
        if origin == Some(bob.id()) && timestamp == now + 1,
        "Sending a new inventory does trigger the relay"
    );

    // Inventory from Eve relayed to Bob.
    alice.receive(
        &eve.addr(),
        Message::Inventory {
            timestamp: now,
            inv,
            origin: None,
        },
    );
    assert_matches!(
        alice.messages(&bob.addr()).next(),
        Some(Message::Inventory { timestamp, origin, .. })
        if origin == Some(eve.id()) && timestamp == now
    );
}

#[test]
fn test_persistent_peer_reconnect() {
    let mut bob = Peer::new("bob", [8, 8, 8, 8], MockStorage::empty());
    let mut eve = Peer::new("eve", [9, 9, 9, 9], MockStorage::empty());
    let mut alice = Peer::config(
        "alice",
        Config {
            connect: vec![bob.addr(), eve.addr()],
            ..Config::default()
        },
        [7, 7, 7, 7],
        vec![],
        MockStorage::empty(),
        fastrand::Rng::new(),
    );

    let mut sim = Simulation::new(
        LocalTime::now(),
        alice.rng.clone(),
        simulator::Options::default(),
    )
    .initialize([&mut alice, &mut bob, &mut eve]);

    sim.run_while([&mut alice, &mut bob, &mut eve], |s| !s.is_settled());

    let ips = alice
        .peers()
        .negotiated()
        .map(|(ip, _)| *ip)
        .collect::<Vec<_>>();
    assert!(ips.contains(&bob.ip));
    assert!(ips.contains(&eve.ip));

    // ... Negotiated ...
    //
    // Now let's disconnect a peer.

    // A transient error such as this will cause Alice to attempt a reconnection.
    let error = Arc::new(io::Error::from(io::ErrorKind::ConnectionReset));

    // A non-transient disconnect, such as one requested by the user will not trigger
    // a reconnection.
    alice.disconnected(
        &eve.addr(),
        nakamoto::DisconnectReason::DialError(error.clone()),
    );
    assert_matches!(alice.outbox().next(), None);

    for _ in 0..MAX_CONNECTION_ATTEMPTS {
        alice.disconnected(
            &bob.addr(),
            nakamoto::DisconnectReason::ConnectionError(error.clone()),
        );
        assert_matches!(alice.outbox().next(), Some(Io::Connect(a)) if a == bob.addr());
        assert_matches!(alice.outbox().next(), None);

        alice.attempted(&bob.addr());
    }

    // After the max connection attempts, a disconnect doesn't trigger a reconnect.
    alice.disconnected(
        &bob.addr(),
        nakamoto::DisconnectReason::ConnectionError(error),
    );
    assert_matches!(alice.outbox().next(), None);
}

#[test]
fn prop_inventory_exchange_dense() {
    fn property(alice_inv: MockStorage, bob_inv: MockStorage, eve_inv: MockStorage) {
        let rng = fastrand::Rng::new();
        let alice = Peer::new("alice", [7, 7, 7, 7], alice_inv.clone());
        let mut bob = Peer::new("bob", [8, 8, 8, 8], bob_inv.clone());
        let mut eve = Peer::new("eve", [9, 9, 9, 9], eve_inv.clone());
        let mut routing = Routing::with_hasher(rng.clone().into());

        for (inv, peer) in &[
            (alice_inv.inventory, alice.id()),
            (bob_inv.inventory, bob.id()),
            (eve_inv.inventory, eve.id()),
        ] {
            for proj in inv {
                routing
                    .entry(proj.id.clone())
                    .or_insert_with(|| HashSet::with_hasher(rng.clone().into()))
                    .insert(*peer);
            }
        }

        // Fully-connected.
        bob.command(Command::Connect(alice.addr()));
        bob.command(Command::Connect(eve.addr()));
        eve.command(Command::Connect(alice.addr()));
        eve.command(Command::Connect(bob.addr()));

        let mut peers: HashMap<_, _> = [(alice.id(), alice), (bob.id(), bob), (eve.id(), eve)]
            .into_iter()
            .collect();
        let mut simulator = Simulation::new(LocalTime::now(), rng, simulator::Options::default())
            .initialize(peers.values_mut());

        simulator.run_while(peers.values_mut(), |s| !s.is_settled());

        for (proj_id, remotes) in &routing {
            for peer in peers.values() {
                let lookup = peer.lookup(proj_id);

                if lookup.local.is_some() {
                    peer.storage()
                        .get(proj_id)
                        .expect("There are no errors querying storage")
                        .expect("The project is available locally");
                } else {
                    for remote in &lookup.remote {
                        peers[remote]
                            .storage()
                            .get(proj_id)
                            .expect("There are no errors querying storage")
                            .expect("The project is available remotely");
                    }
                    assert!(
                        !lookup.remote.is_empty(),
                        "There are remote locations for the project"
                    );
                    assert_eq!(
                        &lookup.remote.into_iter().collect::<HashSet<_>>(),
                        remotes,
                        "The remotes match the global routing table"
                    );
                }
            }
        }
    }
    quickcheck::QuickCheck::new()
        .gen(quickcheck::Gen::new(8))
        .quickcheck(property as fn(MockStorage, MockStorage, MockStorage));
}
