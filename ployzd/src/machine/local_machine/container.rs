//! Machine-local container admission and creation.

use std::path::Path;

use ployz_core::{
    ContainerCreated, ContainerKind, MachineStorageObservation, ProjectName, ResolvedServiceSpec,
    VolumeSource,
};

use super::{Error, LocalMachine};
use crate::docker::ContainerRequest;
use crate::machine::{STORAGE_OBSERVATION_TIMEOUT, local_storage};

impl LocalMachine {
    /// Return fresh local storage evidence for container admission and Global reconciliation.
    pub(crate) async fn observe_storage(&self) -> Option<MachineStorageObservation> {
        local_storage(Path::new("zpool"), STORAGE_OBSERVATION_TIMEOUT).await
    }

    /// Create a container after Machine-local runtime preparation and storage admission.
    ///
    /// # Errors
    ///
    /// Returns when local state, networking, storage, or Docker cannot safely create the
    /// requested container.
    pub(crate) async fn create_container(
        &self,
        kind: ContainerKind,
        project: &ProjectName,
        spec: &ResolvedServiceSpec,
    ) -> Result<ContainerCreated, Error> {
        let network = self.prepare_service_runtime(kind, project, spec).await?;
        let containers = self.containers.as_ref().ok_or(Error::DockerUnavailable)?;
        let record = self.record()?;
        let machine = record.machine().ok_or(Error::NotParticipating)?;
        Ok(containers
            .create_with_network(
                &record.id(),
                machine.subnet.gateway(),
                ContainerRequest {
                    kind,
                    project_name: project,
                    spec,
                    network,
                },
                async || self.ensure_container_storage(spec).await,
            )
            .await?)
    }

    /// Ensure this Machine's Global slot through the shared idempotent Docker path.
    ///
    /// # Errors
    ///
    /// Returns when local state is unavailable, Docker is unavailable, this Machine
    /// is not participating, or Docker cannot ensure the slot.
    pub(crate) async fn ensure_global_slot(
        &self,
        project: &ProjectName,
        spec: &ResolvedServiceSpec,
    ) -> Result<ContainerCreated, Error> {
        let _guard = self.global_slot_lock.lock().await;
        let network = self
            .prepare_service_runtime(ContainerKind::ServiceContainer, project, spec)
            .await?;
        let containers = self.containers.as_ref().ok_or(Error::DockerUnavailable)?;
        let record = self.record()?;
        let machine = record.machine().ok_or(Error::NotParticipating)?;
        Ok(containers
            .ensure_global_slot(
                &record.id(),
                machine.subnet.gateway(),
                project,
                spec,
                network,
                async || self.ensure_container_storage(spec).await,
            )
            .await?)
    }

    async fn ensure_container_storage(
        &self,
        spec: &ResolvedServiceSpec,
    ) -> Result<(), crate::docker::Error> {
        if !has_mounted_provisioned_volume(spec) {
            return Ok(());
        }
        authorize_container_storage(self.observe_storage().await)
    }
}

fn has_mounted_provisioned_volume(spec: &ResolvedServiceSpec) -> bool {
    spec.volume_graph.mounts().iter().any(|mount| {
        matches!(
            spec.volume_graph.volume_for(mount).source,
            VolumeSource::Provisioned { .. }
        )
    })
}

fn authorize_container_storage(
    storage: Option<MachineStorageObservation>,
) -> Result<(), crate::docker::Error> {
    match storage {
        Some(MachineStorageObservation::Ready | MachineStorageObservation::Pool { .. }) => Ok(()),
        Some(MachineStorageObservation::Stateless) => {
            Err(crate::docker::Error::ProvisionedStorageUnsupported)
        }
        None => Err(crate::docker::Error::StorageUnobservable),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        num::NonZeroU64,
        sync::{Arc, Mutex},
    };

    use ployz_core::{
        DockerVolumeName, MachineId, MachineStorageObservation, ProjectName,
        ProvisionedVolumeMaximumBytes, ResolvedServiceSpec, ServiceId, ServiceMode, ServiceMount,
        ServiceName, ServiceVolume, ServiceVolumeGraph, ServiceVolumeReference, VolumeSource,
    };
    use serde_json::json;

    use super::{authorize_container_storage, has_mounted_provisioned_volume};
    use crate::machine::{LocalMachine, LocalMachineError, LocalMachineStore};

    #[tokio::test]
    async fn ensure_global_slot_reports_missing_docker_at_local_machine_seam() {
        let data_dir =
            std::env::temp_dir().join(format!("ployzd-local-global-slot-{}", MachineId::random()));
        let store = Arc::new(Mutex::new(LocalMachineStore::open(&data_dir).unwrap()));
        let (restart, _) = tokio::sync::watch::channel(false);
        let local = LocalMachine::new(store, restart);
        let spec: ResolvedServiceSpec = serde_json::from_value(json!({
            "service_id": ServiceId::random(),
            "name": ServiceName::parse("api").unwrap(),
            "mode": serde_json::to_value(ServiceMode::Global).unwrap(),
            "container": { "image": "example.test/api", "pull_policy": "missing" }
        }))
        .unwrap();

        let error = local
            .ensure_global_slot(&ProjectName::parse("app").unwrap(), &spec)
            .await
            .unwrap_err();

        assert!(matches!(error, LocalMachineError::DockerUnavailable));
        std::fs::remove_dir_all(data_dir).unwrap();
    }

    #[test]
    fn provisioned_mounts_require_current_storage_capability() {
        let pool = MachineStorageObservation::Pool {
            size_bytes: NonZeroU64::new(1).unwrap(),
            used_bytes: 1,
            free_bytes: 0,
        };

        assert!(authorize_container_storage(Some(MachineStorageObservation::Ready)).is_ok());
        assert!(authorize_container_storage(Some(pool)).is_ok());
        assert!(matches!(
            authorize_container_storage(Some(MachineStorageObservation::Stateless)),
            Err(crate::docker::Error::ProvisionedStorageUnsupported)
        ));
        assert!(matches!(
            authorize_container_storage(None),
            Err(crate::docker::Error::StorageUnobservable)
        ));
        assert!(has_mounted_provisioned_volume(&provisioned_spec(true)));
        assert!(!has_mounted_provisioned_volume(&provisioned_spec(false)));
    }

    fn provisioned_spec(mounted: bool) -> ResolvedServiceSpec {
        let mut spec: ResolvedServiceSpec = serde_json::from_value(json!({
            "service_id": ServiceId::random(),
            "name": "api",
            "mode": { "mode": "replicated", "replicas": 1 },
            "container": { "image": "example.test/api", "pull_policy": "missing" }
        }))
        .unwrap();
        let reference = ServiceVolumeReference::parse("data").unwrap();
        let mounts = mounted.then(|| ServiceMount {
            volume: reference.clone(),
            target: ployz_core::ContainerPath::parse("/data").unwrap(),
            read_only: false,
            no_copy: false,
            subpath: None,
        });
        spec.volume_graph = ServiceVolumeGraph::parse(
            vec![ServiceVolume {
                reference,
                source: VolumeSource::Provisioned {
                    name: DockerVolumeName::parse("data").unwrap(),
                    maximum_bytes: ProvisionedVolumeMaximumBytes::new(
                        NonZeroU64::new(1_073_741_824).unwrap(),
                    ),
                    labels: BTreeMap::new(),
                },
            }],
            mounts.into_iter().collect(),
        )
        .unwrap();
        spec
    }
}
