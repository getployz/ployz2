use std::collections::BTreeMap;

use ployz_core::{
    DeployOperation, MachineId, MembershipObservation, RequestedServiceSpec, ServiceMode,
    machine_matches_placement,
};

use super::{DeploySnapshot, PlanError, PlanOptions, eligible_machines};

fn has_unknown(requested: &RequestedServiceSpec, snapshot: &DeploySnapshot) -> bool {
    let Some(capacity) = snapshot.capacity.telemetry() else {
        return false;
    };
    snapshot.machines.iter().any(|machine| {
        machine.membership != MembershipObservation::Down
            && machine_matches_placement(&machine.machine, &requested.placement)
            && !capacity.contains_key(&machine.machine.id)
    })
}

pub(super) fn reject_impossible_replicas(
    requested: &[RequestedServiceSpec],
    snapshot: &DeploySnapshot,
    options: &PlanOptions,
) -> Result<(), PlanError> {
    let Some(capacity) = snapshot.capacity.telemetry() else {
        return Ok(());
    };
    for spec in requested {
        let ServiceMode::Replicated { replicas } = spec.mode else {
            continue;
        };
        if !snapshot.machines.iter().any(|machine| {
            machine.membership != MembershipObservation::Down
                && machine_matches_placement(&machine.machine, &spec.placement)
                && capacity.contains_key(&machine.machine.id)
        }) && has_unknown(spec, snapshot)
        {
            return Err(PlanError::CapacityUnknown);
        }
        let known = eligible_machines(spec, snapshot, options)?;
        let existing = snapshot
            .containers
            .iter()
            .filter(|container| {
                container.resolved_spec.name == spec.name
                    && known
                        .iter()
                        .any(|machine| machine.machine.id == container.machine_id)
            })
            .count() as u64;
        let free = known
            .iter()
            .map(|machine| {
                capacity
                    .get(&machine.machine.id)
                    .expect("eligible capacity Machine has telemetry")
                    .bridge_free_endpoints
            })
            .sum::<u64>();
        if u64::from(replicas.get()) > free.saturating_add(existing) {
            return Err(if has_unknown(spec, snapshot) {
                PlanError::CapacityUnknown
            } else {
                PlanError::InsufficientCapacity
            });
        }
    }
    Ok(())
}

pub(super) fn admit_operations(
    operations: &[DeployOperation],
    requested: &[RequestedServiceSpec],
    snapshot: &DeploySnapshot,
) -> Result<(), PlanError> {
    let Some(capacity) = snapshot.capacity.telemetry() else {
        return Ok(());
    };
    let mut free = capacity
        .iter()
        .map(|(id, telemetry)| (*id, telemetry.bridge_free_endpoints))
        .collect::<BTreeMap<MachineId, _>>();
    for operation in operations {
        let machine_id = match operation {
            DeployOperation::RunContainer { machine_id, .. }
            | DeployOperation::RunHook { machine_id, .. } => machine_id,
            DeployOperation::ReplaceContainer(replacement) => &replacement.machine_id,
            DeployOperation::CreateVolume { .. }
            | DeployOperation::StopContainer { .. }
            | DeployOperation::RemoveContainer { .. }
            | DeployOperation::StopHook { .. }
            | DeployOperation::RemoveVolume { .. } => continue,
        };
        let Some(available) = free.get_mut(machine_id) else {
            return Err(PlanError::CapacityUnknown);
        };
        if *available == 0 {
            let spec = match operation {
                DeployOperation::RunContainer { spec, .. }
                | DeployOperation::RunHook { spec, .. } => requested
                    .iter()
                    .find(|requested| requested.name == spec.name),
                DeployOperation::ReplaceContainer(replacement) => requested
                    .iter()
                    .find(|requested| requested.name == replacement.spec.name),
                DeployOperation::CreateVolume { .. }
                | DeployOperation::StopContainer { .. }
                | DeployOperation::RemoveContainer { .. }
                | DeployOperation::StopHook { .. }
                | DeployOperation::RemoveVolume { .. } => None,
            };
            return Err(if spec.is_some_and(|spec| has_unknown(spec, snapshot)) {
                PlanError::CapacityUnknown
            } else {
                PlanError::InsufficientCapacity
            });
        }
        *available -= 1;
        if matches!(
            operation,
            DeployOperation::RunHook { .. } | DeployOperation::ReplaceContainer(_)
        ) {
            // Both need a spare while the old endpoint still exists, then release it.
            *available += 1;
        }
    }
    Ok(())
}
