//! Store-scoped, nonblocking transcript writer reservations.

use crate::StoreError;
use rustix::fs::{Mode, OFlags, openat};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{File, Metadata, TryLockError};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const FILE: OFlags = OFlags::RDWR
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK)
    .union(OFlags::CLOEXEC);

/// One writer lock, explicitly unlocked on drop without unlinking its file.
pub struct Writer {
    held: File,
    directory: File,
    parent: File,
    database: OsString,
    namespace: OsString,
    name: String,
    identity: (u64, u64),
    path: PathBuf,
}

impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.held.unlock();
    }
}

/// A short-lived mutation guard, shared with other unreserved mutations.
pub struct Mutation {
    _held: Writer,
}

/// Excludes new reservations while maintenance probes and mutates transcripts.
pub(crate) struct Maintenance {
    held: Writer,
}

pub(crate) fn maintenance(
    path: &Path,
    expected: (u64, u64),
) -> Result<Option<Maintenance>, StoreError> {
    acquire(path, expected, "admission.lock".to_owned(), true)
        .map(|held| held.map(|held| Maintenance { held }))
        .map_err(|why| StoreError::Io(path.to_owned(), why))
}

impl Maintenance {
    pub(crate) fn available(&self, session: &str) -> Result<bool, StoreError> {
        if session.is_empty() || session.len() > 1024 {
            return Err(StoreError::Foreign(
                "invalid maintenance transcript key".into(),
            ));
        }
        self.held.check()?;
        acquire(
            &self.held.path,
            self.held.identity,
            format!("{:x}.lock", Sha256::digest(session.as_bytes())),
            true,
        )
        .map(|held| held.is_some())
        .map_err(|why| StoreError::Io(self.held.path.clone(), why))
    }
}

pub(crate) fn identity(path: &Path) -> Result<(u64, u64), StoreError> {
    std::fs::metadata(path)
        .map(|meta| inode(&meta))
        .map_err(|why| StoreError::Io(path.to_owned(), why))
}

fn inode(meta: &Metadata) -> (u64, u64) {
    (meta.dev(), meta.ino())
}

fn refused(why: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, why)
}

fn private(meta: &Metadata, directory: bool) -> std::io::Result<()> {
    let kind = if directory {
        meta.is_dir()
    } else {
        meta.is_file() && meta.nlink() == 1
    };
    if !kind || meta.uid() != rustix::process::getuid().as_raw() || meta.mode() & 0o077 != 0 {
        return Err(refused(
            "writer reservation requires private owner-only files",
        ));
    }
    Ok(())
}

pub(crate) fn reserve(
    path: &Path,
    expected: (u64, u64),
    session: &str,
) -> Result<Option<Writer>, StoreError> {
    if session.is_empty() || session.len() > 1024 {
        return Err(StoreError::Foreign(
            "writer transcript key must contain 1..1024 bytes".into(),
        ));
    }
    acquire_key(path, expected, session, true).map_err(|why| StoreError::Io(path.to_owned(), why))
}

pub(crate) fn mutation(
    path: &Path,
    expected: (u64, u64),
    session: &str,
) -> Result<Option<Mutation>, StoreError> {
    if session.is_empty() || session.len() > 1024 {
        return Err(StoreError::Foreign(
            "writer transcript key must contain 1..1024 bytes".into(),
        ));
    }
    acquire_key(path, expected, session, false)
        .map(|writer| writer.map(|held| Mutation { _held: held }))
        .map_err(|why| StoreError::Io(path.to_owned(), why))
}

fn acquire_key(
    path: &Path,
    expected: (u64, u64),
    session: &str,
    exclusive: bool,
) -> std::io::Result<Option<Writer>> {
    let Some(_admission) = acquire(path, expected, "admission.lock".to_owned(), false)? else {
        return Ok(None);
    };
    acquire(
        path,
        expected,
        format!("{:x}.lock", Sha256::digest(session.as_bytes())),
        exclusive,
    )
}

fn acquire(
    path: &Path,
    expected: (u64, u64),
    name: String,
    exclusive: bool,
) -> std::io::Result<Option<Writer>> {
    let canonical = std::fs::canonicalize(path)?;
    let database = canonical
        .file_name()
        .ok_or_else(|| refused("missing database name"))?
        .to_owned();
    let parent = File::from(rustix::fs::open(
        canonical
            .parent()
            .ok_or_else(|| refused("missing store parent"))?,
        DIRECTORY,
        Mode::empty(),
    )?);
    let opened = File::from(openat(&parent, &database, FILE, Mode::empty())?);
    let metadata = opened.metadata()?;
    private(&metadata, false)?;
    if inode(&metadata) != expected {
        return Err(refused("transcript store identity changed"));
    }
    let mut namespace = database.clone();
    namespace.push(".writers");
    match rustix::fs::mkdirat(&parent, &namespace, Mode::from_raw_mode(0o700)) {
        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
        Err(why) => return Err(why.into()),
    }
    let directory = File::from(openat(&parent, &namespace, DIRECTORY, Mode::empty())?);
    private(&directory.metadata()?, true)?;
    let held = File::from(openat(
        &directory,
        name.as_str(),
        FILE | OFlags::CREATE,
        Mode::from_raw_mode(0o600),
    )?);
    private(&held.metadata()?, false)?;
    let locked = if exclusive {
        held.try_lock()
    } else {
        held.try_lock_shared()
    };
    match locked {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(None),
        Err(TryLockError::Error(why)) => return Err(why),
    }
    let writer = Writer {
        held,
        directory,
        parent,
        database,
        namespace,
        name,
        identity: expected,
        path: canonical,
    };
    writer.validate()?;
    Ok(Some(writer))
}

impl Writer {
    pub(crate) fn matches(&self, identity: (u64, u64), session: &str) -> bool {
        self.identity == identity
            && self.name == format!("{:x}.lock", Sha256::digest(session.as_bytes()))
    }

    /// Refuse a replaced database, reservation directory, or lock-file identity.
    pub fn check(&self) -> Result<(), StoreError> {
        self.validate()
            .map_err(|why| StoreError::Io(self.path.clone(), why))
    }

    fn validate(&self) -> std::io::Result<()> {
        let database = File::from(openat(&self.parent, &self.database, FILE, Mode::empty())?);
        let meta = database.metadata()?;
        private(&meta, false)?;
        if inode(&meta) != self.identity {
            return Err(refused("transcript store identity changed"));
        }
        let directory = File::from(openat(
            &self.parent,
            &self.namespace,
            DIRECTORY,
            Mode::empty(),
        )?);
        let meta = directory.metadata()?;
        private(&meta, true)?;
        if inode(&meta) != inode(&self.directory.metadata()?) {
            return Err(refused("writer reservation directory changed"));
        }
        let file = File::from(openat(&directory, self.name.as_str(), FILE, Mode::empty())?);
        let meta = file.metadata()?;
        private(&meta, false)?;
        if inode(&meta) != inode(&self.held.metadata()?) {
            return Err(refused("writer reservation file changed"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
