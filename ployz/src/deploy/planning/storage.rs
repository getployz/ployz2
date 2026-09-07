//! Plan-wide provisioning budgets and capacity-aware volume placement.

use crate::deploy::{DeployOperation, DeploySnapshot, PlanError};
use ployz_core::{
    DockerVolumeName, MachineId, MachineObservation, ProvisionedVolumeMaximumBytes,
    RawVolumeSource, ResolvedServiceSpec, ServiceVolume, StorageBudget, StorageCapacity,
    StorageCapacityError,
};
use std::collections::BTreeMap;

type Volumes = BTreeMap<DockerVolumeName, ProvisionedVolumeMaximumBytes>;

pub(super) fn bounds<'volume>(volumes: impl Iterator<Item = &'volume ServiceVolume>) -> Volumes {
    volumes
        .filter_map(|volume| match volume.source.kind() {
            RawVolumeSource::Provisioned {
                name,
                maximum_bytes,
                ..
            } => Some((name.clone(), *maximum_bytes)),
            RawVolumeSource::Bind { .. }
            | RawVolumeSource::External { .. }
            | RawVolumeSource::Ordinary { .. }
            | RawVolumeSource::Tmpfs { .. } => None,
        })
        .collect()
}

/// Dataset inventory must be known before absence can authorize a new placement.
///
/// # Errors
/// Returns a Machine-specific unknown-capacity error for failed or omitted observations.
pub(super) fn capacity<'snapshot>(
    snapshot: &'snapshot DeploySnapshot,
    machine: &MachineObservation,
) -> Result<&'snapshot StorageCapacity, PlanError> {
    match snapshot.storage_capacity.get(&machine.machine.id) {
        Some(Ok(capacity)) => Ok(capacity),
        failure => Err(PlanError::Storage {
            machine_id: machine.machine.id,
            machine: machine.machine.name.clone(),
            source: StorageCapacityError::StorageCapacityUnknown {
                message: match failure {
                    Some(Err(error)) => error.message.clone(),
                    _ => "the Machine did not return fresh storage capacity".into(),
                },
            },
        }),
    }
}

pub(super) fn budget(
    snapshot: &DeploySnapshot,
    machine: &MachineObservation,
    volumes: &Volumes,
) -> Result<StorageBudget, PlanError> {
    capacity(snapshot, machine)?
        .budget(volumes)
        .map_err(|source| PlanError::Storage {
            machine_id: machine.machine.id,
            machine: machine.machine.name.clone(),
            source,
        })
}

pub(super) fn budgets(
    operations: &[DeployOperation],
    snapshot: &DeploySnapshot,
) -> Result<Vec<ployz_core::MachineStorageBudget>, PlanError> {
    let mut by_machine = BTreeMap::<MachineId, Volumes>::new();
    for operation in operations {
        if let Some(spec) = operation.spec() {
            let volumes = bounds(spec.volume_graph().mounted_volumes());
            if !volumes.is_empty() {
                by_machine
                    .entry(operation.machine_id())
                    .or_default()
                    .extend(volumes);
            }
        }
    }
    by_machine
        .into_iter()
        .map(|(id, volumes)| {
            let machine = snapshot
                .machines
                .iter()
                .find(|machine| machine.machine.id == id)
                .expect("planned Machine belongs to snapshot");
            Ok(ployz_core::MachineStorageBudget {
                machine_id: id,
                machine_name: machine.machine.name.clone(),
                budget: budget(snapshot, machine, &volumes)?,
            })
        })
        .collect()
}

/// Full resolved placements, deduplicated across hooks and replacement operations.
pub(crate) fn preparations(
    operations: &[DeployOperation],
) -> BTreeMap<MachineId, Vec<ResolvedServiceSpec>> {
    let mut by_machine = BTreeMap::<MachineId, Vec<ResolvedServiceSpec>>::new();
    for operation in operations {
        if let Some(spec) = operation
            .spec()
            .filter(|spec| spec.volume_graph().has_mounted_provisioned_volume())
        {
            let specs = by_machine.entry(operation.machine_id()).or_default();
            if !specs.contains(spec) {
                specs.push(spec.clone());
            }
        }
    }
    by_machine
}

/// Put storage preparation before hooks, stops, and application creation.
pub(crate) fn prepend_preparations(operations: &mut Vec<DeployOperation>) {
    let preparations = preparations(operations)
        .into_iter()
        .map(|(machine_id, specs)| DeployOperation::PrepareVolumes { machine_id, specs });
    operations.splice(..0, preparations);
}
