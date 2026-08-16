use std::collections::{BTreeMap, BTreeSet};

use ployz_core::{
    ContainerKind, ContainerObservation, ContainerRuntimeObservation, DockerVolumeId,
    DockerVolumeName, HostBind, MachineId, MachineObservation, MembershipObservation,
    PortPublication, RequestedServiceSpec, ResolvedServiceSpec, ResolvedUpdateConfig, ServiceId,
    ServiceMode, ServiceVolume, SpecChange, UpdateOrder, VolumeSource, compare_specs,
    machine_matches_selector, same_service_mode_kind,
};

use super::{
    DeployOperation, DeployPlan, DeploySnapshot, ObservedDockerVolume, PlanError, PlanOptions,
    ReplacementOperation,
};

/// Cross-spec Docker Volume placement decided once for a compose project.
///
/// Compose computes this through [`claim_shared_volumes`] and passes it into
/// [`plan_deploy`]. The Deploy Snapshot is not mutated.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VolumeClaims {
    /// Hide other Machines' copies of this Docker Volume name (shared-replicated anchor).
    retain: BTreeMap<DockerVolumeName, MachineId>,
    /// Extra Docker Volume observations this Service should see.
    overlay_for: BTreeMap<String, Vec<ObservedDockerVolume>>,
    /// Requested Service Specs already normalized by [`claim_shared_volumes`].
    normalized: BTreeMap<String, RequestedServiceSpec>,
    /// CreateVolume operations for shared-replicated Docker Volumes.
    pub volume_operations: Vec<DeployOperation>,
}

/// Normalize each Requested Service Spec once, then claim shared Docker Volumes.
///
/// Compose calls this instead of crate-visible eligibility helpers or snapshot
/// mutation. The returned claims feed [`plan_deploy`] and, when present, skip a
/// second normalize.
pub fn claim_shared_volumes<'a>(
    specs: impl IntoIterator<Item = &'a RequestedServiceSpec>,
    snapshot: &DeploySnapshot,
    options: PlanOptions,
) -> Result<VolumeClaims, PlanError> {
    let specs = specs
        .into_iter()
        .map(normalize_and_validate)
        .collect::<Result<Vec<_>, _>>()?;
    let volume_uses = named_volume_uses(&specs);
    let mut claims = VolumeClaims::default();
    for spec in &specs {
        claims
            .normalized
            .insert(spec.name.to_string(), spec.clone());
    }
    claims.volume_operations =
        claim_shared_replicated(&volume_uses, snapshot, options, &mut claims)?;
    claim_shared_global_overlays(&specs, &volume_uses, snapshot, options, &mut claims)?;
    Ok(claims)
}

pub fn plan_deploy(
    requested: &RequestedServiceSpec,
    snapshot: &DeploySnapshot,
    new_service_id: ServiceId,
    options: PlanOptions,
    claims: &VolumeClaims,
) -> Result<DeployPlan, PlanError> {
    // TODO(UT-009): preserve the missing within-spec port-conflict validation.
    let normalized_owned;
    let requested = match claims.normalized.get(requested.name.as_str()) {
        Some(normalized) => normalized,
        None => {
            normalized_owned = normalize_and_validate(requested)?;
            &normalized_owned
        }
    };
    // A Deploy is a finite calculation over this supplied snapshot, never a reconciliation loop.
    let mut machines = eligible_machines(requested, snapshot, options);
    let volume_operations = volume_operations(requested, snapshot, &mut machines, claims)?;

    let matching_service_ids = snapshot
        .containers
        .iter()
        .filter(|container| container.service_name == requested.name)
        .map(|container| container.service_id)
        .collect::<BTreeSet<_>>();
    let (service_id, is_new_service) = match matching_service_ids.len() {
        0 => (new_service_id, true),
        1 => (
            matching_service_ids
                .into_iter()
                .next()
                .ok_or(PlanError::NoEligibleMachines)?,
            false,
        ),
        _ => {
            return Err(PlanError::AmbiguousService {
                matches: matching_service_ids.into_iter().collect(),
            });
        }
    };
    if !is_new_service
        && snapshot.containers.iter().any(|container| {
            container.service_id == service_id
                && !same_service_mode_kind(&container.resolved_spec.mode, &requested.mode)
        })
    {
        return Err(PlanError::ServiceModeCannotChange);
    }

    let mut service_operations = match requested.mode {
        ServiceMode::Replicated { replicas } => plan_replicated(
            requested,
            snapshot,
            &service_id,
            machines,
            replicas.get() as usize,
            options,
        ),
        ServiceMode::Global => plan_global(requested, snapshot, &service_id, machines, options),
    };
    let mut operations =
        pre_deploy_operations(requested, snapshot, &service_id, &service_operations);
    operations.append(&mut service_operations);

    Ok(DeployPlan::new(
        service_id,
        is_new_service,
        volume_operations,
        operations,
    ))
}

#[derive(Clone, Copy)]
struct NamedVolumeUse<'a> {
    service_name: &'a str,
    service: &'a RequestedServiceSpec,
    volume: &'a ServiceVolume,
    global: bool,
}

fn named_volume_uses(
    specs: &[RequestedServiceSpec],
) -> BTreeMap<DockerVolumeName, Vec<NamedVolumeUse<'_>>> {
    let mut uses = BTreeMap::<DockerVolumeName, Vec<NamedVolumeUse<'_>>>::new();
    for service in specs {
        let service_name = service.name.as_str();
        for mount in &service.mounts {
            let Some(volume) = service
                .volumes
                .iter()
                .find(|volume| volume.reference == mount.volume)
            else {
                continue;
            };
            let VolumeSource::Named { name, .. } = &volume.source else {
                continue;
            };
            let uses = uses.entry(name.clone()).or_default();
            if !uses
                .iter()
                .any(|volume_use| volume_use.service_name == service_name)
            {
                uses.push(NamedVolumeUse {
                    service_name,
                    service,
                    volume,
                    global: matches!(service.mode, ServiceMode::Global),
                });
            }
        }
    }
    uses
}

fn claim_shared_replicated(
    volume_uses: &BTreeMap<DockerVolumeName, Vec<NamedVolumeUse<'_>>>,
    snapshot: &DeploySnapshot,
    options: PlanOptions,
    claims: &mut VolumeClaims,
) -> Result<Vec<DeployOperation>, PlanError> {
    let mut operations = Vec::new();
    let mut remaining = volume_uses
        .iter()
        .filter(|(_, uses)| uses.len() > 1 && uses.iter().all(|volume_use| !volume_use.global))
        .collect::<Vec<_>>();
    while !remaining.is_empty() {
        let mut component = vec![remaining.remove(0)];
        while let Some(index) = remaining.iter().position(|(_, candidate_uses)| {
            candidate_uses.iter().any(|candidate| {
                component.iter().any(|(_, component_uses)| {
                    component_uses
                        .iter()
                        .any(|volume_use| volume_use.service_name == candidate.service_name)
                })
            })
        }) {
            component.push(remaining.remove(index));
        }
        let services = component
            .iter()
            .flat_map(|(_, uses)| uses.iter())
            .map(|volume_use| (volume_use.service_name, volume_use.service))
            .collect::<BTreeMap<_, _>>();
        let mut services = services.into_iter();
        let (_first_service_name, first_service) = services
            .next()
            .expect("shared Volume component has at least two services");
        let mut eligible = volume_eligible_machine_ids(first_service, snapshot, options)?;
        for (_, service) in services {
            let other_eligible = volume_eligible_machine_ids(service, snapshot, options)?;
            eligible.retain(|machine_id| other_eligible.contains(machine_id));
        }
        if eligible.is_empty() {
            return Err(PlanError::NoEligibleMachines);
        }
        let machine_id = eligible.remove(0);
        for (name, uses) in component {
            claims.retain.insert(name.clone(), machine_id);
            if !snapshot
                .volumes
                .iter()
                .any(|volume| volume.id.machine_id == machine_id && volume.id.name == *name)
            {
                let first_use = uses.first().expect("shared Volume has at least two uses");
                if let Some(observed) = observed_from_create(machine_id, first_use.volume) {
                    for spec_name in claims.normalized.keys() {
                        claims
                            .overlay_for
                            .entry(spec_name.clone())
                            .or_default()
                            .push(observed.clone());
                    }
                }
                operations.push(DeployOperation::CreateVolume {
                    machine_id,
                    volume: first_use.volume.clone(),
                });
            }
        }
    }
    Ok(operations)
}

fn claim_shared_global_overlays(
    specs: &[RequestedServiceSpec],
    volume_uses: &BTreeMap<DockerVolumeName, Vec<NamedVolumeUse<'_>>>,
    snapshot: &DeploySnapshot,
    options: PlanOptions,
    claims: &mut VolumeClaims,
) -> Result<(), PlanError> {
    let shared_global = volume_uses
        .iter()
        .any(|(_, uses)| uses.len() > 1 && uses.iter().all(|volume_use| volume_use.global));
    if !shared_global {
        return Ok(());
    }
    let mut extra = Vec::new();
    for spec in specs {
        let name = spec.name.as_str();
        claims
            .overlay_for
            .entry(name.to_owned())
            .or_default()
            .extend(extra.iter().cloned());
        let mut machines = eligible_machines(spec, snapshot, options);
        for operation in volume_operations(spec, snapshot, &mut machines, claims)? {
            if let DeployOperation::CreateVolume { machine_id, volume } = operation
                && let Some(observed) = observed_from_create(machine_id, &volume)
            {
                extra.push(observed);
            }
        }
    }
    Ok(())
}

fn volume_eligible_machine_ids(
    requested: &RequestedServiceSpec,
    snapshot: &DeploySnapshot,
    options: PlanOptions,
) -> Result<Vec<MachineId>, PlanError> {
    let mut machines = eligible_machines(requested, snapshot, options);
    volume_constraints(requested, snapshot, &mut machines, &VolumeClaims::default())?;
    Ok(machines
        .into_iter()
        .map(|machine| machine.machine.id)
        .collect())
}

fn visible_volumes<'a>(
    snapshot: &'a DeploySnapshot,
    claims: &'a VolumeClaims,
    service_name: &str,
) -> Vec<&'a ObservedDockerVolume> {
    let overlay = claims
        .overlay_for
        .get(service_name)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    snapshot
        .volumes
        .iter()
        .filter(|observed| {
            claims
                .retain
                .get(&observed.id.name)
                .is_none_or(|machine_id| observed.id.machine_id == *machine_id)
        })
        .chain(overlay.iter())
        .collect()
}

fn observed_from_create(
    machine_id: MachineId,
    volume: &ServiceVolume,
) -> Option<ObservedDockerVolume> {
    let VolumeSource::Named { name, driver, .. } = &volume.source else {
        return None;
    };
    Some(ObservedDockerVolume {
        id: DockerVolumeId {
            machine_id,
            name: name.clone(),
        },
        driver: driver
            .as_ref()
            .map_or_else(|| "local".into(), |driver| driver.name.clone()),
        options: driver
            .as_ref()
            .map_or_else(Default::default, |driver| driver.options.clone()),
    })
}

fn eligible_machines<'a>(
    requested: &RequestedServiceSpec,
    snapshot: &'a DeploySnapshot,
    options: PlanOptions,
) -> Vec<&'a MachineObservation> {
    let mut machines = snapshot
        .machines
        .iter()
        .filter(|machine| machine.membership != MembershipObservation::Down)
        .filter(|machine| {
            requested.placement.machines.is_empty()
                || requested
                    .placement
                    .machines
                    .iter()
                    .any(|selector| machine_matches_selector(&machine.machine, selector))
        })
        .collect::<Vec<_>>();
    shuffle(&mut machines, options.placement_seed);
    machines
}

fn volume_operations(
    requested: &RequestedServiceSpec,
    snapshot: &DeploySnapshot,
    machines: &mut Vec<&MachineObservation>,
    claims: &VolumeClaims,
) -> Result<Vec<DeployOperation>, PlanError> {
    // TODO(UT-001, UT-007, UT-008, UT-051, UT-052, UT-078): preserve the baseline
    // placement/pull ceiling: do not filter by memory, image platform, or local image presence,
    // and do not pull images from other Machines.
    let (mounted_volumes, missing_volumes) =
        volume_constraints(requested, snapshot, machines, claims)?;
    let volumes = visible_volumes(snapshot, claims, requested.name.as_str());
    let mut operations = Vec::new();
    match requested.mode {
        ServiceMode::Replicated { .. } if !missing_volumes.is_empty() => {
            // TODO(UT-005): named-volume driver and label options remain part of planned creation;
            // revisit only if Ployz changes to externally managed volumes exclusively.
            let machine_id = machines
                .first()
                .map(|machine| machine.machine.id)
                .ok_or(PlanError::NoEligibleMachines)?;
            machines.retain(|machine| machine.machine.id == machine_id);
            operations.extend(
                missing_volumes
                    .into_iter()
                    .map(|volume| DeployOperation::CreateVolume { machine_id, volume }),
            );
        }
        ServiceMode::Global => {
            for machine in machines.iter() {
                operations.extend(
                    mounted_volumes
                        .iter()
                        .filter(|volume| {
                            !volumes.iter().any(|observed| {
                                observed.id.machine_id == machine.machine.id
                                    && volume_matches(observed, volume)
                            })
                        })
                        .map(|volume| DeployOperation::CreateVolume {
                            machine_id: machine.machine.id,
                            volume: volume.clone(),
                        }),
                );
            }
        }
        ServiceMode::Replicated { .. } => {}
    }
    Ok(operations)
}

fn volume_constraints(
    requested: &RequestedServiceSpec,
    snapshot: &DeploySnapshot,
    machines: &mut Vec<&MachineObservation>,
    claims: &VolumeClaims,
) -> Result<(Vec<ServiceVolume>, Vec<ServiceVolume>), PlanError> {
    let volumes = visible_volumes(snapshot, claims, requested.name.as_str());
    let mounted_volumes = mounted_named_volumes(requested)?
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let mut missing_volumes = Vec::new();
    for volume in &mounted_volumes {
        machines.retain(|machine| {
            !volumes.iter().any(|observed| {
                observed.id.machine_id == machine.machine.id
                    && volume_has_same_name(observed, volume)
                    && !volume_matches(observed, volume)
            })
        });
        if matches!(requested.mode, ServiceMode::Replicated { .. }) {
            let locations = volumes
                .iter()
                .filter(|observed| volume_matches(observed, volume))
                .map(|observed| &observed.id.machine_id)
                .collect::<BTreeSet<_>>();
            if !locations.is_empty() {
                machines.retain(|machine| locations.contains(&machine.machine.id));
            } else {
                missing_volumes.push(volume.clone());
            }
        }
    }
    if machines.is_empty() {
        // TODO(UT-079, UT-093): keep the baseline's coarse constraint failure until a
        // separately approved diagnostic contract defines a detailed report.
        return Err(PlanError::NoEligibleMachines);
    }
    Ok((mounted_volumes, missing_volumes))
}

fn normalize_and_validate(
    requested: &RequestedServiceSpec,
) -> Result<RequestedServiceSpec, PlanError> {
    let mut normalized = requested.clone();
    normalized.caddy_config = normalized
        .caddy_config
        .take()
        .map(|config| config.trim().to_owned())
        .filter(|config| !config.is_empty());

    let mut volumes = BTreeSet::new();
    for volume in &normalized.volumes {
        if !volumes.insert(volume.reference.clone()) {
            return Err(PlanError::DuplicateVolumeReference {
                reference: volume.reference.clone(),
            });
        }
    }
    for mount in &normalized.mounts {
        if !volumes.contains(&mount.volume) {
            return Err(PlanError::UnknownVolumeReference {
                reference: mount.volume.clone(),
            });
        }
    }

    let mut configs = BTreeSet::new();
    for config in &normalized.configs {
        if !configs.insert(config.name.as_str()) {
            return Err(PlanError::DuplicateConfigName {
                name: config.name.clone(),
            });
        }
    }
    for mount in &normalized.container.config_mounts {
        if !configs.contains(mount.config_name.as_str()) {
            return Err(PlanError::UnknownConfigName {
                name: mount.config_name.clone(),
            });
        }
    }
    Ok(normalized)
}

fn shuffle<T>(values: &mut [T], mut state: u64) {
    for upper in (1..values.len()).rev() {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut random = state;
        random = (random ^ (random >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        random = (random ^ (random >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        random ^= random >> 31;
        values.swap(upper, random as usize % (upper + 1));
    }
}

fn volume_has_same_name(observed: &ObservedDockerVolume, volume: &ServiceVolume) -> bool {
    let VolumeSource::Named { name, .. } = &volume.source else {
        return false;
    };
    observed.id.name == *name
}

fn volume_matches(observed: &ObservedDockerVolume, volume: &ServiceVolume) -> bool {
    let VolumeSource::Named { name, driver, .. } = &volume.source else {
        return false;
    };
    observed.id.name == *name
        && driver.as_ref().is_none_or(|required| {
            required.name == observed.driver && required.options == observed.options
        })
}

fn pre_deploy_operations(
    requested: &RequestedServiceSpec,
    snapshot: &DeploySnapshot,
    service_id: &ServiceId,
    service_operations: &[DeployOperation],
) -> Vec<DeployOperation> {
    if requested.pre_deploy.is_none() {
        return Vec::new();
    }
    let target = service_operations
        .iter()
        .find_map(|operation| match operation {
            DeployOperation::RunContainer {
                machine_id, spec, ..
            }
            | DeployOperation::ReplaceContainer(ReplacementOperation {
                machine_id, spec, ..
            }) => Some((machine_id, spec)),
            DeployOperation::CreateVolume { .. }
            | DeployOperation::StopContainer { .. }
            | DeployOperation::RemoveContainer { .. }
            | DeployOperation::StopHook { .. }
            | DeployOperation::RunHook { .. }
            | DeployOperation::Sequence { .. } => None,
        });
    let Some((machine_id, spec)) = target else {
        return Vec::new();
    };

    let hooks = snapshot
        .containers
        .iter()
        .filter(|container| {
            container.service_id == *service_id && container.kind == ContainerKind::PreDeployHook
        })
        .collect::<Vec<_>>();
    let mut operations = hooks
        .iter()
        .filter(|container| super::is_active_runtime(&container.runtime))
        .map(|container| DeployOperation::StopHook {
            machine_id: container.machine_id,
            container_id: container.container_id,
        })
        .collect::<Vec<_>>();
    operations.push(DeployOperation::RunHook {
        machine_id: *machine_id,
        spec: spec.clone(),
        old_hook_containers: hooks
            .into_iter()
            .map(|container| (container.machine_id, container.container_id))
            .collect(),
    });
    operations
}

fn mounted_named_volumes(
    requested: &RequestedServiceSpec,
) -> Result<Vec<&ServiceVolume>, PlanError> {
    let mut by_docker_name = BTreeMap::<DockerVolumeName, &ServiceVolume>::new();
    for mount in &requested.mounts {
        let Some(volume) = requested
            .volumes
            .iter()
            .find(|volume| volume.reference == mount.volume)
        else {
            continue;
        };
        let VolumeSource::Named { name, .. } = &volume.source else {
            continue;
        };
        if let Some(existing) = by_docker_name.get(name) {
            if existing.source != volume.source {
                return Err(PlanError::ConflictingDockerVolumeDefinitions { name: name.clone() });
            }
        } else {
            by_docker_name.insert(name.clone(), volume);
        }
    }
    Ok(by_docker_name.into_values().collect())
}

fn has_mounted_named_volume(requested: &RequestedServiceSpec) -> bool {
    requested.mounts.iter().any(|mount| {
        requested.volumes.iter().any(|volume| {
            volume.reference == mount.volume && matches!(volume.source, VolumeSource::Named { .. })
        })
    })
}

fn plan_global(
    requested: &RequestedServiceSpec,
    snapshot: &DeploySnapshot,
    service_id: &ServiceId,
    machines: Vec<&MachineObservation>,
    options: PlanOptions,
) -> Vec<DeployOperation> {
    let current = service_containers(snapshot, service_id);
    let mut used = BTreeSet::new();
    let mut operations = Vec::new();

    for machine in machines {
        let on_machine = current
            .iter()
            .copied()
            .filter(|(_, container)| container.machine_id == machine.machine.id)
            .collect::<Vec<_>>();
        if let Some((kept_index, _)) = on_machine
            .iter()
            .copied()
            .find(|(_, container)| is_up_to_date(container, requested, options))
        {
            used.insert(kept_index);
            continue;
        }

        if let Some((replaced_index, container)) = on_machine
            .iter()
            .copied()
            .find(|(_, container)| super::is_active_runtime(&container.runtime))
        {
            used.insert(replaced_index);
            for (index, other) in on_machine.iter().copied() {
                if index != replaced_index
                    && super::is_active_runtime(&other.runtime)
                    && other.resolved_spec.ports.iter().any(|old| {
                        requested
                            .ports
                            .iter()
                            .any(|new| host_ports_conflict(old, new))
                    })
                {
                    operations.push(DeployOperation::StopContainer {
                        machine_id: machine.machine.id,
                        container_id: other.container_id,
                    });
                }
            }
            let order = determine_update_order(container, requested);
            operations.push(DeployOperation::ReplaceContainer(ReplacementOperation {
                machine_id: machine.machine.id,
                old_container_id: container.container_id,
                spec: resolve(requested, *service_id, order),
                skip_health_monitor: options.skip_health_monitor,
            }));
        } else {
            operations.push(DeployOperation::RunContainer {
                machine_id: machine.machine.id,
                spec: resolve(
                    requested,
                    *service_id,
                    requested.update.order.unwrap_or(UpdateOrder::StartFirst),
                ),
                skip_health_monitor: options.skip_health_monitor,
            });
        }
    }

    remove_unused(&mut operations, current, &used);
    operations
}

fn plan_replicated(
    requested: &RequestedServiceSpec,
    snapshot: &DeploySnapshot,
    service_id: &ServiceId,
    mut machines: Vec<&MachineObservation>,
    replicas: usize,
    options: PlanOptions,
) -> Vec<DeployOperation> {
    let current = service_containers(snapshot, service_id);
    let mut by_machine = BTreeMap::<MachineId, Vec<(usize, &ContainerObservation)>>::new();
    for (index, container) in &current {
        by_machine
            .entry(container.machine_id)
            .or_default()
            .push((*index, container));
    }
    for containers in by_machine.values_mut() {
        containers.sort_by_key(|(_, container)| is_up_to_date(container, requested, options));
    }
    machines.sort_by_key(|machine| {
        let containers = by_machine.get(&machine.machine.id);
        let up_to_date = containers
            .into_iter()
            .flatten()
            .filter(|(_, container)| is_up_to_date(container, requested, options))
            .count();
        (
            std::cmp::Reverse(up_to_date),
            std::cmp::Reverse(containers.map_or(0, Vec::len)),
        )
    });

    let mut used = BTreeSet::new();
    let mut operations = Vec::new();
    for machine in machines.iter().cycle().take(replicas) {
        let existing = by_machine
            .get_mut(&machine.machine.id)
            .and_then(Vec::pop)
            .map(|(index, container)| {
                used.insert(index);
                container
            });
        match existing {
            Some(container) if is_up_to_date(container, requested, options) => {}
            Some(container) => {
                let order = determine_update_order(container, requested);
                operations.push(DeployOperation::ReplaceContainer(ReplacementOperation {
                    machine_id: machine.machine.id,
                    old_container_id: container.container_id,
                    spec: resolve(requested, *service_id, order),
                    skip_health_monitor: options.skip_health_monitor,
                }));
            }
            None => operations.push(DeployOperation::RunContainer {
                machine_id: machine.machine.id,
                spec: resolve(
                    requested,
                    *service_id,
                    requested.update.order.unwrap_or(UpdateOrder::StartFirst),
                ),
                skip_health_monitor: options.skip_health_monitor,
            }),
        }
    }
    remove_unused(&mut operations, current, &used);
    operations
}

fn service_containers<'a>(
    snapshot: &'a DeploySnapshot,
    service_id: &ServiceId,
) -> Vec<(usize, &'a ContainerObservation)> {
    snapshot
        .containers
        .iter()
        .enumerate()
        .filter(|(_, container)| {
            container.service_id == *service_id && container.kind == ContainerKind::ServiceContainer
        })
        .collect()
}

fn remove_unused(
    operations: &mut Vec<DeployOperation>,
    current: Vec<(usize, &ContainerObservation)>,
    used: &BTreeSet<usize>,
) {
    for (index, container) in current {
        if !used.contains(&index) {
            // TODO(UT-075): placement changes remove now-ineligible containers; there is no
            // deploy-time Machine filter that leaves excluded containers running.
            operations.push(DeployOperation::RemoveContainer {
                machine_id: container.machine_id,
                container_id: container.container_id,
            });
        }
    }
}

fn is_up_to_date(
    container: &ContainerObservation,
    requested: &RequestedServiceSpec,
    options: PlanOptions,
) -> bool {
    !options.force_recreate
        && is_running(container)
        && compare_specs(&container.resolved_spec, requested) == SpecChange::UpToDate
}

fn is_running(container: &ContainerObservation) -> bool {
    matches!(
        container.runtime,
        ContainerRuntimeObservation::Running { .. }
    )
}

fn determine_update_order(
    current: &ContainerObservation,
    requested: &RequestedServiceSpec,
) -> UpdateOrder {
    if let Some(order) = requested.update.order {
        return order;
    }
    if current.resolved_spec.ports.iter().any(|old| {
        requested
            .ports
            .iter()
            .any(|new| host_ports_conflict(old, new))
    }) {
        return UpdateOrder::StopFirst;
    }
    if (matches!(requested.mode, ServiceMode::Global)
        || matches!(
            requested.mode,
            ServiceMode::Replicated { replicas } if replicas.get() == 1
        ))
        && has_mounted_named_volume(requested)
    {
        return UpdateOrder::StopFirst;
    }
    UpdateOrder::StartFirst
}

fn host_ports_conflict(left: &PortPublication, right: &PortPublication) -> bool {
    let (
        PortPublication::Host {
            bind: left_bind,
            published_port: left_port,
            transport_protocol: left_protocol,
            ..
        },
        PortPublication::Host {
            bind: right_bind,
            published_port: right_port,
            transport_protocol: right_protocol,
            ..
        },
    ) = (left, right)
    else {
        return false;
    };
    left_port == right_port
        && left_protocol == right_protocol
        && binds_overlap(left_bind, right_bind)
}

fn binds_overlap(left: &HostBind, right: &HostBind) -> bool {
    match (left, right) {
        (HostBind::All, _) | (_, HostBind::All) => true,
        (HostBind::Address { address: left }, HostBind::Address { address: right }) => {
            left == right
        }
        (HostBind::Address { address }, HostBind::Prefix { prefix })
        | (HostBind::Prefix { prefix }, HostBind::Address { address }) => prefix.contains(address),
        (HostBind::Prefix { prefix: left }, HostBind::Prefix { prefix: right }) => {
            left.contains(&right.network()) || right.contains(&left.network())
        }
    }
}

#[must_use]
fn resolve(
    requested: &RequestedServiceSpec,
    service_id: ServiceId,
    order: UpdateOrder,
) -> ResolvedServiceSpec {
    ResolvedServiceSpec {
        service_id,
        name: requested.name.clone(),
        mode: requested.mode.clone(),
        container: requested.container.clone(),
        placement: requested.placement.clone(),
        ports: requested.ports.clone(),
        volumes: requested.volumes.clone(),
        mounts: requested.mounts.clone(),
        configs: requested.configs.clone(),
        pre_deploy: requested.pre_deploy.clone(),
        caddy_config: requested.caddy_config.clone(),
        update: ResolvedUpdateConfig {
            order,
            monitor_millis: requested.update.monitor_millis,
        },
    }
}
