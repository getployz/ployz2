//! Machine-local effect for retiring a definitely ineligible Global slot.

use ployz_core::ContainerId;

use super::{Error, LocalMachine};

impl LocalMachine {
    /// Stop and remove one local Global slot without removing its volumes.
    ///
    /// # Errors
    ///
    /// Returns an error when Docker is unavailable or the Container cannot be stopped or removed.
    pub(crate) async fn retire_global_slot(&self, container_id: &ContainerId) -> Result<(), Error> {
        let _guard = self.global_slot_lock.lock().await;
        let containers = self.containers.as_ref().ok_or(Error::DockerUnavailable)?;
        containers.stop(container_id, None, None).await?;
        Ok(containers.remove(container_id, false, false).await?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ployz_core::{ContainerId, MachineId};

    use super::*;
    use crate::machine::LocalMachineStore;

    #[tokio::test]
    async fn retirement_reports_missing_docker() {
        let data_dir = std::env::temp_dir().join(format!(
            "ployzd-local-global-slot-retire-{}",
            MachineId::random()
        ));
        let store = Arc::new(Mutex::new(LocalMachineStore::open(&data_dir).unwrap()));
        let (restart, _) = tokio::sync::watch::channel(false);
        let local = LocalMachine::new(store, restart);
        let container_id = ContainerId::parse("1".repeat(64)).unwrap();

        let error = local.retire_global_slot(&container_id).await.unwrap_err();

        assert!(matches!(error, Error::DockerUnavailable));
        std::fs::remove_dir_all(data_dir).unwrap();
    }
}
