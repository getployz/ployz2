use std::collections::BTreeMap;

use ployz_core::{
    MachineId, MembershipObservation, ProjectName, RequestedServiceSpec, ServiceMode,
    machine_matches_placement,
};

use super::{DeploySnapshot, PlanError, PlanOptions, eligible_machines};

fn has_unknown(requested: &RequestedServiceSpec, snapshot: &DeploySnapshot) -> bool {
    let Some(capacity) = &snapshot.capacity else {
        return false;
    };
    snapshot.machines.iter().any(|machine| {
        machine.membership != MembershipObservation::Down
            && machine_matches_placement(&machine.machine, &requested.placement)
            && !capacity.contains_key(&machine.machine.id)
    })
}

pub(super) fn reject_impossible_replicas(
    project_name: &ProjectName,
    requested: &[RequestedServiceSpec],
    snapshot: &DeploySnapshot,
    options: &PlanOptions,
) -> Result<(), PlanError> {
    let Some(capacity) = &snapshot.capacity else {
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
                container.project_name == *project_name
                    && container.resolved_spec.name == spec.name
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

pub(super) struct CapacityBudget<'snapshot> {
    snapshot: &'snapshot DeploySnapshot,
    free: Option<BTreeMap<MachineId, u64>>,
}

impl<'snapshot> CapacityBudget<'snapshot> {
    pub(super) fn from_snapshot(snapshot: &'snapshot DeploySnapshot) -> Self {
        Self {
            snapshot,
            free: snapshot.capacity.as_ref().map(|capacity| {
                capacity
                    .iter()
                    .map(|(id, telemetry)| (*id, telemetry.bridge_free_endpoints))
                    .collect()
            }),
        }
    }

    pub(super) fn fits(&self, machine_id: &MachineId, peak: u64) -> bool {
        self.free
            .as_ref()
            .is_none_or(|free| free.get(machine_id).is_some_and(|free| *free >= peak))
    }

    pub(super) fn consume(&mut self, machine_id: &MachineId, persistent: u64) {
        if let Some(free) = self.free.as_mut() {
            *free
                .get_mut(machine_id)
                .expect("capacity candidates have fresh telemetry") -= persistent;
        }
    }

    pub(super) fn error(&self, requested: &RequestedServiceSpec) -> PlanError {
        if has_unknown(requested, self.snapshot) {
            PlanError::CapacityUnknown
        } else {
            PlanError::InsufficientCapacity
        }
    }
}
