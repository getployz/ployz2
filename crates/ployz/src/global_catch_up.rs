//! Global catch-up: place observed eligible Globals onto this Machine only.

use std::collections::BTreeMap;

use ployz_core::{
    BridgeEndpointCapacity, ContainerCreated, ContainerId, ContainerKind, ContainerObservation,
    CreateContainerRequest, InspectRequest, ListContainersRequest, LiveServices, Machine,
    MachineId, MachineStorageObservation, MachineTarget, ObservedGlobalSlotSpec, QualifiedService,
    ResolvedServiceSpec, RpcError, ServiceObservation, ServicePlacementEligibility, op,
    service_containers,
};

use crate::{connect::Client, deploy::endpoint_capacity_error, failure::Failure};

/// Catch-up failed after membership committed.
#[derive(Debug)]
pub(crate) struct CatchUpError {
    cause: Failure,
    unresolved: Vec<QualifiedService>,
}

impl CatchUpError {
    /// Record the failure and Globals whose eligibility or running slot is unresolved.
    pub(crate) fn new(cause: Failure, unresolved: Vec<QualifiedService>) -> Self {
        Self { cause, unresolved }
    }
}

pub(crate) trait CatchUpClient {
    async fn live_services(&mut self) -> Result<LiveServices<RpcError>, Failure>;
    async fn target_storage(
        &mut self,
        machine_id: &MachineId,
    ) -> Result<Option<MachineStorageObservation>, Failure>;
    async fn bridge_capacity(
        &mut self,
        machine_id: &MachineId,
    ) -> Result<Option<BridgeEndpointCapacity>, Failure>;
    async fn create_slot(
        &mut self,
        machine_id: &MachineId,
        request: CreateContainerRequest,
    ) -> Result<ContainerCreated, RpcError>;
    async fn start_slot(
        &mut self,
        machine_id: &MachineId,
        container_id: ContainerId,
    ) -> Result<(), RpcError>;
    /// List Containers directly from the joined Machine for final verification.
    async fn target_containers(
        &mut self,
        machine_id: &MachineId,
    ) -> Result<Vec<ContainerObservation>, Failure>;
}

impl CatchUpClient for Client {
    async fn live_services(&mut self) -> Result<LiveServices<RpcError>, Failure> {
        Client::live_services(self).await.map_err(Into::into)
    }

    async fn target_storage(
        &mut self,
        machine_id: &MachineId,
    ) -> Result<Option<MachineStorageObservation>, Failure> {
        self.read::<op::Inspect>(
            InspectRequest {
                include_storage: true,
                ..Default::default()
            },
            &MachineTarget::from(machine_id),
        )
        .await
        .map(|details| details.storage)
        .map_err(Into::into)
    }

    async fn bridge_capacity(
        &mut self,
        machine_id: &MachineId,
    ) -> Result<Option<BridgeEndpointCapacity>, Failure> {
        let details = self
            .read::<op::Inspect>(
                InspectRequest {
                    telemetry: ployz_core::InspectTelemetry::BridgeCapacity,
                    ..Default::default()
                },
                &MachineTarget::from(machine_id),
            )
            .await
            .map_err(Failure::from)?;
        Ok(details.telemetry.map(|telemetry| telemetry.into_bridge()))
    }

    async fn create_slot(
        &mut self,
        machine_id: &MachineId,
        request: CreateContainerRequest,
    ) -> Result<ContainerCreated, RpcError> {
        let target = MachineTarget::from(machine_id);
        let details = self
            .read::<op::Inspect>(
                InspectRequest {
                    include_storage: true,
                    ..Default::default()
                },
                &target,
            )
            .await?;
        let machine = details
            .machine
            .filter(|machine| {
                machine.id == *machine_id
                    && details.phase == ployz_core::LocalMachinePhase::Participating
            })
            .ok_or_else(|| RpcError {
                code: ployz_core::RpcErrorCode::Conflict,
                message: "Global catch-up target has no participating Machine observation".into(),
                details: serde_json::Value::Null,
            })?;
        let eligibility = request.resolved_spec.placement_eligibility_in_project(
            &request.project_name,
            &machine,
            details.storage.as_ref(),
        );
        if eligibility != ServicePlacementEligibility::Eligible {
            if matches!(eligibility, ServicePlacementEligibility::Ineligible(_)) {
                let containers = self
                    .read::<op::ListContainers>(ListContainersRequest {}, &target)
                    .await?;
                for container in containers.containers.into_iter().filter(|container| {
                    container.machine_id == *machine_id
                        && container.kind == ContainerKind::ServiceContainer
                        && container.project_name == request.project_name
                        && container.resolved_spec.name == request.resolved_spec.name
                }) {
                    self.call::<op::StopContainer>(
                        ployz_core::StopContainerRequest {
                            container_id: container.container_id,
                            signal: None,
                            grace_period_seconds: None,
                        },
                        Some(&target),
                    )
                    .await
                    .map_err(RpcError::from)?;
                    self.call::<op::RemoveContainer>(
                        ployz_core::RemoveContainerRequest {
                            container_id: container.container_id,
                            remove_volumes: false,
                            force: false,
                        },
                        Some(&target),
                    )
                    .await
                    .map_err(RpcError::from)?;
                }
            }
            return Err(RpcError {
                code: ployz_core::RpcErrorCode::Conflict,
                message: format!("Global catch-up target eligibility is {eligibility:?}"),
                details: serde_json::Value::Null,
            });
        }
        // Explicit Deploy replacement keys also distinguish the previous Container.
        // Reuse its exact persisted creation when catch-up finds it before Start.
        let containers = self
            .read::<op::ListContainers>(ListContainersRequest {}, &target)
            .await?;
        if let Some(existing) = containers.containers.into_iter().find(|container| {
            container.machine_id == *machine_id
                && container.kind == ContainerKind::ServiceContainer
                && container.project_name == request.project_name
                && container.resolved_spec == request.resolved_spec
        }) {
            return Ok(ContainerCreated {
                container_id: existing.container_id,
                display_name: existing.display_name.clone(),
            });
        }
        self.call::<op::CreateContainer>(request, Some(&target))
            .await
            .map_err(Into::into)
    }

    async fn start_slot(
        &mut self,
        machine_id: &MachineId,
        container_id: ContainerId,
    ) -> Result<(), RpcError> {
        self.call::<op::StartContainer>(
            ployz_core::StartContainerRequest { container_id },
            Some(&MachineTarget::from(machine_id)),
        )
        .await
        .map(|_| ())
        .map_err(Into::into)
    }

    async fn target_containers(
        &mut self,
        machine_id: &MachineId,
    ) -> Result<Vec<ContainerObservation>, Failure> {
        self.read::<op::ListContainers>(ListContainersRequest {}, &MachineTarget::from(machine_id))
            .await
            .map(|list| list.containers)
            .map_err(Failure::from)
    }
}

pub(crate) fn joined_catch_up_error(error: CatchUpError) -> String {
    let mut message = format!(
        "Machine joined, but Global catch-up is incomplete; it remains a Cluster member. {}",
        error.cause
    );
    if !error.unresolved.is_empty() {
        message.push_str("\nGlobals requiring attention:");
        for identity in error.unresolved {
            if identity == QualifiedService::system_ingress() {
                message.push_str("\n- ployz-system/ingress: run `ployz ingress deploy`.");
            } else {
                message.push_str(&format!(
                    "\n- {identity}: redeploy Project Service `{identity}`."
                ));
            }
        }
    }
    message
}

/// Globals this Machine is eligible for and does not already run.
#[must_use]
pub fn plan_global_catch_up(
    services: &[ServiceObservation],
    this_machine: &Machine,
    storage: Option<&MachineStorageObservation>,
) -> Vec<ObservedGlobalSlotSpec> {
    services
        .iter()
        .filter_map(|service| {
            let slot = eligible_catch_up_slot(service, this_machine, storage)?;
            (!slot.is_running_on(&service.containers, this_machine)).then_some(slot)
        })
        .collect()
}

fn eligible_catch_up_slot(
    service: &ServiceObservation,
    machine: &Machine,
    storage: Option<&MachineStorageObservation>,
) -> Option<ObservedGlobalSlotSpec> {
    let slot = service.observed_global_slot()?;
    (slot.resolved_spec().placement_eligibility_in_project(
        &service.identity.project,
        machine,
        storage,
    ) == ServicePlacementEligibility::Eligible)
        .then_some(slot)
}

/// Copy every observed eligible Global onto `this_machine` only.
///
/// # Errors
///
/// Fails when listing Services fails, the target Machine does not answer, or
/// any eligible Global cannot be placed, or required storage evidence is unknown.
pub(crate) async fn catch_up_globals<C: CatchUpClient>(
    client: &mut C,
    this_machine: &Machine,
) -> Result<(), CatchUpError> {
    let live = client
        .live_services()
        .await
        .map_err(|error| CatchUpError::new(error, Vec::new()))?;
    if !live.containers.all_targets_succeeded() {
        return Err(CatchUpError::new(
            Failure::usage(format!(
                "Global catch-up cannot plan from partial Service observations: {}; restore peer connectivity and redeploy",
                crate::failure::partial_failure_details(&live.containers)
            )),
            Vec::new(),
        ));
    }
    let services = live.services();
    let needs_storage = services
        .iter()
        .filter_map(ServiceObservation::observed_global_slot)
        .any(|slot| {
            matches!(
                slot.resolved_spec().placement_eligibility_in_project(
                    &slot.identity().project,
                    this_machine,
                    None
                ),
                ServicePlacementEligibility::Unknown(_)
            )
        });
    let storage_result = if needs_storage {
        client.target_storage(&this_machine.id).await
    } else {
        Ok(None)
    };
    let storage = storage_result.as_ref().ok().and_then(Option::as_ref);
    let unknown = services
        .iter()
        .filter_map(ServiceObservation::observed_global_slot)
        .filter(|slot| {
            matches!(
                slot.resolved_spec().placement_eligibility_in_project(
                    &slot.identity().project,
                    this_machine,
                    storage
                ),
                ServicePlacementEligibility::Unknown(_)
            )
        })
        .map(|slot| slot.identity().clone())
        .collect::<Vec<_>>();
    let initially_eligible = services
        .iter()
        .filter_map(|service| eligible_catch_up_slot(service, this_machine, storage))
        .map(|slot| (slot.identity().clone(), slot))
        .collect::<BTreeMap<_, _>>();
    let slots = plan_global_catch_up(&services, this_machine, storage);
    let initially_missing: Vec<_> = slots
        .iter()
        .map(|slot| slot.identity().clone())
        .chain(unknown.iter().cloned())
        .collect();
    let endpoint_creates = slots
        .iter()
        .filter(|slot| {
            !service_has_slot(
                &services,
                this_machine,
                slot.identity(),
                slot.resolved_spec(),
            )
        })
        .count();
    if endpoint_creates > 0 {
        let capacity = client
            .bridge_capacity(&this_machine.id)
            .await
            .map_err(|error| CatchUpError::new(error, initially_missing.clone()))?;
        if let Some(error) = endpoint_capacity_error(endpoint_creates, capacity.as_ref()) {
            return Err(CatchUpError::new(
                Failure::usage(error.to_string()),
                initially_missing,
            ));
        }
    }
    if !slots.is_empty() {
        eprintln!("Placing Global Services on this Machine.");
    }
    let mut failures = unknown
        .iter()
        .map(|identity| {
            (
                identity.clone(),
                match &storage_result {
                    Err(error) => format!("storage eligibility is unknown: {error}"),
                    Ok(_) => {
                        "storage eligibility is unknown; restore storage evidence and redeploy"
                            .to_owned()
                    }
                },
            )
        })
        .collect::<Vec<_>>();
    for slot in slots {
        let (identity, resolved_spec) = slot.into_parts();
        let failure_identity = identity.clone();
        let created = client
            .create_slot(
                &this_machine.id,
                CreateContainerRequest {
                    creation_key: Some(crate::cluster::global_creation_key(&resolved_spec)),
                    kind: ContainerKind::ServiceContainer,
                    project_name: identity.project,
                    resolved_spec,
                },
            )
            .await;
        let result = match created {
            Ok(created) => {
                client
                    .start_slot(&this_machine.id, created.container_id)
                    .await
            }
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            failures.push((failure_identity, error.to_string()));
        }
    }
    let missing_if_unverified = initially_eligible
        .keys()
        .cloned()
        .chain(unknown.iter().cloned())
        .collect();
    let target_containers = client
        .target_containers(&this_machine.id)
        .await
        .map_err(|error| CatchUpError::new(error, missing_if_unverified))?;
    let target_services = service_containers(target_containers);
    let missing = initially_eligible
        .into_iter()
        .filter_map(|(identity, slot)| {
            (!slot.is_running_on(&target_services, this_machine)).then_some(identity)
        })
        .chain(unknown)
        .collect::<Vec<_>>();
    if !missing.is_empty() || !failures.is_empty() {
        let details = failures
            .iter()
            .map(|(identity, error)| format!("{identity}: {error}"))
            .collect::<Vec<_>>()
            .join("; ");
        let cause = if details.is_empty() {
            Failure::usage("eligible Globals are not running after catch-up")
        } else {
            Failure::usage(format!("Global catch-up incomplete: {details}"))
        };
        return Err(CatchUpError::new(cause, missing));
    }
    Ok(())
}

fn service_has_slot(
    services: &[ServiceObservation],
    machine: &Machine,
    identity: &QualifiedService,
    spec: &ResolvedServiceSpec,
) -> bool {
    let wanted = spec.serving_shape();
    services
        .iter()
        .flat_map(|service| &service.containers)
        .any(|container| {
            let observation = container.as_observation();
            &observation.identity() == identity
                && observation.machine_id == machine.id
                && observation.resolved_spec.serving_shape() == wanted
        })
}

#[cfg(test)]
#[path = "global_catch_up_tests.rs"]
mod tests;
