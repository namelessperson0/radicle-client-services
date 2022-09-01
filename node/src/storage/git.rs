use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fmt, fs, io};

use git_ref_format::refspec;
use git_url::Url;
use once_cell::sync::Lazy;
use radicle_git_ext as git_ext;

pub use radicle_git_ext::Oid;

use crate::collections::HashMap;
use crate::crypto::Signer;
use crate::git;
use crate::identity::{ProjId, UserId};

use super::{
    Error, Inventory, ReadRepository, ReadStorage, Remote, Remotes, Unverified, WriteRepository,
    WriteStorage,
};

pub static RADICLE_ID_REF: Lazy<refspec::PatternString> =
    Lazy::new(|| refspec::pattern!("heads/radicle/id"));
pub static REMOTES_GLOB: Lazy<refspec::PatternString> =
    Lazy::new(|| refspec::pattern!("refs/remotes/*"));

pub struct Storage {
    path: PathBuf,
    signer: Arc<dyn Signer>,
}

impl fmt::Debug for Storage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Storage(..)")
    }
}

impl ReadStorage for Storage {
    fn user_id(&self) -> &UserId {
        self.signer.public_key()
    }

    fn url(&self) -> Url {
        Url {
            scheme: git_url::Scheme::File,
            host: Some(self.path.to_string_lossy().to_string()),
            ..Url::default()
        }
    }

    fn get(&self, _id: &ProjId) -> Result<Option<Remotes<Unverified>>, Error> {
        todo!()
    }

    fn inventory(&self) -> Result<Inventory, Error> {
        self.projects()
    }
}

impl WriteStorage for Storage {
    type Repository = Repository;

    fn repository(&self, proj: &ProjId) -> Result<Self::Repository, Error> {
        Repository::open(self.path.join(proj.to_string()))
    }
}

impl Storage {
    pub fn open<P: AsRef<Path>>(path: P, signer: impl Signer) -> Result<Self, io::Error> {
        let path = path.as_ref().to_path_buf();

        match fs::create_dir_all(&path) {
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
            Ok(()) => {}
        }

        Ok(Self {
            path,
            signer: Arc::new(signer),
        })
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    pub fn signer(&self) -> Arc<dyn Signer> {
        self.signer.clone()
    }

    pub fn projects(&self) -> Result<Vec<ProjId>, Error> {
        let mut projects = Vec::new();

        for result in fs::read_dir(&self.path)? {
            let path = result?;
            let id = ProjId::try_from(path.file_name())?;

            projects.push(id);
        }
        Ok(projects)
    }
}

pub struct Repository {
    pub(crate) backend: git2::Repository,
}

impl Repository {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let backend = match git2::Repository::open_bare(path.as_ref()) {
            Err(e) if git_ext::is_not_found_err(&e) => {
                let backend = git2::Repository::init_opts(
                    path,
                    git2::RepositoryInitOptions::new()
                        .bare(true)
                        .no_reinit(true)
                        .external_template(false),
                )?;

                Ok(backend)
            }
            Ok(repo) => Ok(repo),
            Err(e) => Err(e),
        }?;

        Ok(Self { backend })
    }

    pub fn find_reference(&self, remote: &UserId, name: &str) -> Result<Oid, Error> {
        let name = format!("refs/remotes/{}/{}", remote, name);
        let target = self
            .backend
            .find_reference(&name)?
            .target()
            .ok_or(Error::InvalidRef)?;

        Ok(target.into())
    }
}

impl ReadRepository for Repository {
    fn path(&self) -> &Path {
        self.backend.path()
    }

    fn remote(&self, user: &UserId) -> Result<Remote<Unverified>, Error> {
        // TODO: Only fetch standard refs.
        let entries = self
            .backend
            .references_glob(format!("refs/remotes/{}/*", user).as_str())?;
        let mut refs = HashMap::default();

        for e in entries {
            let e = e?;
            let name = e.name().ok_or(Error::InvalidRef)?;
            let (_, refname) = git::parse_ref::<UserId>(name)?;
            let oid = e.target().ok_or(Error::InvalidRef)?;

            refs.insert(refname.to_string(), oid.into());
        }
        Ok(Remote::new(*user, refs))
    }

    fn remotes(&self) -> Result<Remotes<Unverified>, Error> {
        let refs = self.backend.references_glob(REMOTES_GLOB.as_str())?;
        let mut remotes = HashMap::default();

        for r in refs {
            let r = r?;
            let name = r.name().ok_or(Error::InvalidRef)?;
            let (id, refname) = git::parse_ref::<UserId>(name)?;
            let entry = remotes
                .entry(id)
                .or_insert_with(|| Remote::new(id, HashMap::default()));
            let oid = r.target().ok_or(Error::InvalidRef)?;

            entry.refs.insert(refname.to_string(), oid.into());
        }
        Ok(Remotes::new(remotes))
    }
}

impl WriteRepository for Repository {
    /// Fetch all remotes of a project from the given URL.
    fn fetch(&mut self, url: &Url) -> Result<(), git2::Error> {
        // TODO: Have function to fetch specific remotes.
        // TODO: Return meaningful info on success.
        //
        // Repository layout should look like this:
        //
        //   /refs/remotes/<remote>
        //         /heads
        //           /master
        //         /tags
        //         ...
        //
        let url = url.to_string();
        let refs: &[&str] = &["refs/remotes/*:refs/remotes/*"];
        let mut remote = self.backend.remote_anonymous(&url)?;
        let mut opts = git2::FetchOptions::default();

        // TODO: Make sure we verify before pruning, as pruning may get us into
        // a state we can't roll back.
        opts.prune(git2::FetchPrune::On);
        remote.fetch(refs, Some(&mut opts), None)?;

        Ok(())
    }
}

impl From<git2::Repository> for Repository {
    fn from(backend: git2::Repository) -> Self {
        Self { backend }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git;
    use crate::storage::{ReadStorage, WriteRepository};
    use crate::test::crypto::MockSigner;
    use crate::test::fixtures;
    use git_url::Url;

    #[test]
    fn test_list_remotes() {
        let dir = tempfile::tempdir().unwrap();
        let storage = fixtures::storage(dir.path());
        let inv = storage.inventory().unwrap();
        let proj = inv.first().unwrap();
        let refs = git::list_remotes(&Url {
            host: Some(dir.path().to_string_lossy().to_string()),
            scheme: git_url::Scheme::File,
            path: format!("/{}", proj).into(),
            ..Url::default()
        })
        .unwrap();

        let remotes = storage.repository(proj).unwrap().remotes().unwrap();

        assert_eq!(refs, remotes);
    }

    #[test]
    fn test_fetch() {
        let tmp = tempfile::tempdir().unwrap();
        let alice = fixtures::storage(tmp.path().join("alice"));
        let bob = Storage::open(tmp.path().join("bob"), MockSigner::default()).unwrap();
        let inventory = alice.inventory().unwrap();
        let proj = inventory.first().unwrap();
        let remotes = alice.repository(proj).unwrap().remotes().unwrap();
        let refname = "heads/master";

        // Have Bob fetch Alice's refs.
        bob.repository(proj)
            .unwrap()
            .fetch(&Url {
                host: Some(alice.path().to_string_lossy().to_string()),
                scheme: git_url::Scheme::File,
                path: format!("/{}", proj).into(),
                ..Url::default()
            })
            .unwrap();

        for (id, _) in remotes.into_iter() {
            let alice_oid = alice
                .repository(proj)
                .unwrap()
                .find_reference(&id, refname)
                .unwrap();
            let bob_oid = bob
                .repository(proj)
                .unwrap()
                .find_reference(&id, refname)
                .unwrap();

            assert_eq!(alice_oid, bob_oid);
        }
    }
}
