//! Cross-process exclusion between installation and Machine-local mutations.

use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};

use thiserror::Error;

const LOCK_FILE: &str = ".install.lock";
const ACTIVE_FILE: &str = ".upgrade-active";

#[derive(Clone, Debug)]
pub struct Admission {
    lock_dir: PathBuf,
    data_dir: PathBuf,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("a Ployz installation or upgrade is active")]
    Busy,
    #[error("Machine admission: {0}")]
    Io(#[from] io::Error),
}

pub struct MutationGuard(File);
pub struct InstallGuard(File);

impl Admission {
    #[must_use]
    pub fn new(lock_dir: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            lock_dir: lock_dir.into(),
            data_dir: data_dir.into(),
        }
    }

    pub fn try_mutation(&self) -> Result<MutationGuard, Error> {
        let file = self.open_lock()?;
        file.try_lock_shared().map_err(lock_error)?;
        if self.active_path().is_file() {
            let _ = file.unlock();
            return Err(Error::Busy);
        }
        Ok(MutationGuard(file))
    }

    #[must_use]
    pub fn lock_dir(&self) -> &std::path::Path {
        &self.lock_dir
    }

    pub fn try_install(&self) -> Result<InstallGuard, Error> {
        let file = self.open_lock()?;
        file.try_lock().map_err(lock_error)?;
        Ok(InstallGuard(file))
    }

    pub fn lock_install(&self) -> Result<InstallGuard, Error> {
        let file = self.open_lock()?;
        file.lock()?;
        Ok(InstallGuard(file))
    }

    #[must_use]
    pub fn active(&self) -> bool {
        self.active_path().is_file()
    }

    pub(super) fn mark_active(&self, attempt_id: &str) -> Result<(), Error> {
        fs::create_dir_all(&self.data_dir)?;
        crate::filesystem::atomic_write(&self.active_path(), attempt_id.as_bytes(), 0o600)?;
        Ok(())
    }

    pub(super) fn clear_active(&self, attempt_id: &str) -> Result<(), Error> {
        let path = self.active_path();
        match fs::read_to_string(&path) {
            Ok(active) if active == attempt_id => fs::remove_file(path)?,
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        File::open(&self.data_dir)?.sync_all()?;
        Ok(())
    }

    fn open_lock(&self) -> Result<File, Error> {
        fs::create_dir_all(&self.lock_dir)?;
        fs::set_permissions(&self.lock_dir, fs::Permissions::from_mode(0o750))?;
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(self.lock_dir.join(LOCK_FILE))
            .map_err(Into::into)
    }

    fn active_path(&self) -> PathBuf {
        self.data_dir.join(ACTIVE_FILE)
    }
}

fn lock_error(error: TryLockError) -> Error {
    match error {
        TryLockError::WouldBlock => Error::Busy,
        TryLockError::Error(error) => Error::Io(error),
    }
}

impl Drop for MutationGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_and_active_handoff_exclude_mutations_across_instances() {
        let root = crate::test_dir::TestDir::new("installer-admission");
        let first = Admission::new(root.0.join("run"), root.0.join("data"));
        let second = Admission::new(root.0.join("run"), root.0.join("data"));

        let install = first.try_install().unwrap();
        assert!(matches!(second.try_mutation(), Err(Error::Busy)));
        first.mark_active("attempt").unwrap();
        drop(install);
        assert!(matches!(second.try_mutation(), Err(Error::Busy)));

        let install = second.try_install().unwrap();
        second.clear_active("attempt").unwrap();
        drop(install);
        assert!(second.try_mutation().is_ok());
    }
}
