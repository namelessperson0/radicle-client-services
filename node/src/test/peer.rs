use std::net;
use std::ops::{Deref, DerefMut};

use log::*;
use nakamoto_net::simulator;
use nakamoto_net::Protocol as _;

use crate::address_book::{KnownAddress, Source};
use crate::clock::RefClock;
use crate::collections::HashMap;
use crate::decoder::Decoder;
use crate::protocol::*;
use crate::storage::{ReadStorage, WriteStorage};
use crate::*;

/// Protocol instantiation used for testing.
pub type Protocol<S> = crate::protocol::Protocol<HashMap<net::IpAddr, KnownAddress>, S>;

#[derive(Debug)]
pub struct Peer<S> {
    pub name: &'static str,
    pub protocol: Protocol<S>,
    pub ip: net::IpAddr,
    pub rng: fastrand::Rng,
    pub local_time: LocalTime,
    pub local_addr: net::SocketAddr,

    initialized: bool,
}

impl<S> simulator::Peer<Protocol<S>> for Peer<S>
where
    S: ReadStorage + WriteStorage + 'static,
{
    fn init(&mut self) {
        self.initialize()
    }

    fn addr(&self) -> net::SocketAddr {
        net::SocketAddr::new(self.ip, DEFAULT_PORT)
    }
}

impl<S> Deref for Peer<S> {
    type Target = Protocol<S>;

    fn deref(&self) -> &Self::Target {
        &self.protocol
    }
}

impl<S> DerefMut for Peer<S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.protocol
    }
}

impl<S> Peer<S>
where
    S: ReadStorage + WriteStorage + 'static,
{
    pub fn new(name: &'static str, ip: impl Into<net::IpAddr>, storage: S) -> Self {
        Self::config(
            name,
            Config::default(),
            ip,
            vec![],
            storage,
            fastrand::Rng::new(),
        )
    }

    pub fn config(
        name: &'static str,
        config: Config,
        ip: impl Into<net::IpAddr>,
        addrs: Vec<(net::SocketAddr, Source)>,
        storage: S,
        rng: fastrand::Rng,
    ) -> Self {
        let addrs = addrs
            .into_iter()
            .map(|(addr, src)| (addr.ip(), KnownAddress::new(addr, src, None)))
            .collect();
        let local_time = LocalTime::now();
        let clock = RefClock::from(local_time);
        let protocol = Protocol::new(config, clock, storage, addrs, rng.clone());
        let ip = ip.into();
        let local_addr = net::SocketAddr::new(ip, rng.u16(..));

        Self {
            name,
            protocol,
            ip,
            local_addr,
            rng,
            local_time,
            initialized: false,
        }
    }

    pub fn initialize(&mut self) {
        if !self.initialized {
            info!("{}: Initializing: address = {}", self.name, self.ip);

            self.initialized = true;
            self.protocol.initialize(LocalTime::now());
        }
    }

    pub fn receive(&mut self, peer: &net::SocketAddr, msg: Message) {
        let bytes = serde_json::to_vec(&Envelope {
            magic: NETWORK_MAGIC,
            msg,
        })
        .unwrap();

        self.protocol.received_bytes(peer, &bytes);
    }

    pub fn connect_from(&mut self, remote: &net::SocketAddr) {
        let local = net::SocketAddr::new(self.ip, self.rng.u16(..));

        self.initialize();
        self.protocol.connected(*remote, &local, Link::Inbound);
        self.receive(remote, Message::hello());
        self.receive(
            remote,
            Message::Inventory {
                seq: 0,
                inv: vec![],
            },
        );

        let mut msgs = self.messages(remote);
        msgs.find(|m| matches!(m, Message::Hello { .. }))
            .expect("`hello` is sent");
        msgs.find(|m| matches!(m, Message::GetInventory { .. }))
            .expect("`get-inventory` is sent");
    }

    pub fn connect_to(&mut self, remote: &net::SocketAddr) {
        self.initialize();
        self.protocol.attempted(remote);
        self.protocol
            .connected(*remote, &self.local_addr, Link::Outbound);

        let mut msgs = self.messages(remote);
        msgs.find(|m| matches!(m, Message::Hello { .. }))
            .expect("`hello` is sent");
        msgs.find(|m| matches!(m, Message::GetInventory { .. }))
            .expect("`get-inventory` is sent");

        self.receive(remote, Message::hello());
    }

    /// Get outgoing messages sent from this peer to the remote address.
    pub fn messages(&mut self, remote: &net::SocketAddr) -> impl Iterator<Item = Message> {
        let mut stream = Decoder::<Envelope>::new(2048);
        let mut msgs = Vec::new();

        for o in self.protocol.outbox().iter() {
            match o {
                Io::Write(a, bytes) if a == remote => {
                    stream.input(bytes);
                }
                _ => {}
            }
        }

        while let Some(envelope) = stream.decode_next().unwrap() {
            msgs.push(envelope.msg);
        }
        msgs.into_iter()
    }

    /// Get a draining iterator over the peers's I/O outbox.
    pub fn outbox(&mut self) -> impl Iterator<Item = Io<(), DisconnectReason>> + '_ {
        self.protocol.outbox().drain(..)
    }
}
