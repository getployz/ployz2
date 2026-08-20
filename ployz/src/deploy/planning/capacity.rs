//! Endpoint-capacity filtering and plan-wide budgeting.

use std::collections::BTreeMap;

use ployz_core::{
    BridgeEndpointCapacity, MachineId, MembershipObservation, RequestedServiceSpec,
    machine_matches_placement,
};

use super::{DeploySnapshot, PlanError};

pub(crate) fn endpoint_capacity_error(
    required: usize,
    capacity: Option<&BridgeEndpointCapacity>,
) -> Option<PlanError> {
    match capacity {
        None if required > 0 => Some(PlanError::CapacityUnknown),
        Some(capacity) if capacity.free_endpoints() < required as u64 => {
            Some(PlanError::InsufficientCapacity)
        }
        None | Some(_) => None,
    }
}

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

#[derive(Clone, Copy)]
pub(super) struct EndpointDemand {
    peak: u64,
    persistent: u64,
    uses_hook: bool,
}

#[derive(Clone, Copy)]
pub(super) enum EndpointOperation {
    Unchanged,
    Create,
    Replace,
}

impl EndpointDemand {
    pub(super) fn for_operation(operation: EndpointOperation, hook_pending: bool) -> Self {
        let changes = !matches!(operation, EndpointOperation::Unchanged);
        let uses_hook = changes && hook_pending;
        Self {
            peak: u64::from(changes) + u64::from(uses_hook),
            persistent: u64::from(matches!(operation, EndpointOperation::Create))
                + u64::from(uses_hook),
            uses_hook,
        }
    }

    pub(super) fn uses_hook(self) -> bool {
        self.uses_hook
    }
}

#[derive(Clone)]
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
                    .map(|(id, capacity)| (*id, capacity.free_endpoints()))
                    .collect()
            }),
        }
    }

    pub(super) fn fits(&self, machine_id: &MachineId, peak: u64) -> bool {
        peak == 0
            || self
                .free
                .as_ref()
                .is_none_or(|free| free.get(machine_id).is_some_and(|free| *free >= peak))
    }

    pub(super) fn fits_demand(&self, machine_id: &MachineId, demand: EndpointDemand) -> bool {
        self.fits(machine_id, demand.peak)
    }

    pub(super) fn reserve(&mut self, machine_id: &MachineId, demand: EndpointDemand) -> bool {
        if !self.fits_demand(machine_id, demand) {
            return false;
        }
        if demand.persistent > 0
            && let Some(free) = self.free.as_mut()
        {
            *free
                .get_mut(machine_id)
                .expect("capacity candidates have fresh telemetry") -= demand.persistent;
        }
        true
    }

    pub(super) fn error(&self, requested: &RequestedServiceSpec) -> PlanError {
        if has_unknown(requested, self.snapshot) {
            PlanError::CapacityUnknown
        } else {
            PlanError::InsufficientCapacity
        }
    }

    pub(super) fn error_for<'a>(
        &self,
        machine_ids: impl IntoIterator<Item = &'a MachineId>,
    ) -> PlanError {
        if self.free.as_ref().is_some_and(|free| {
            machine_ids
                .into_iter()
                .any(|machine_id| !free.contains_key(machine_id))
        }) {
            PlanError::CapacityUnknown
        } else {
            PlanError::InsufficientCapacity
        }
    }
}
