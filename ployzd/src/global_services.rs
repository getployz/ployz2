//! Run this Machine's share of the Cluster's Global Services.
//!
//! A Global Service is declared to run one Container per Machine, but nothing
//! evaluates that: `ServiceMode` is never read outside the CLI, and placement
//! is resolved once by whoever deploys and frozen into the Container set. A
//! Machine joining afterwards receives nothing. Caddy is the one that bites,
//! because `ployz cloud enroll` deploys it on the founder while the founder is
//! still alone, so every Cluster grown through Cloud had ingress on one host.
//!
//! Placement for a Global Service is decidable locally — this Machine needs
//! exactly one Container of each, and needs to know nothing about the other
//! Machines to work that out. So each Machine converges only itself, with no
//! Cluster-wide redeploy and no coordination.
//!
//! Driven by the Container subscription rather than a one-shot at join,
//! because the founder holds its Register before it deploys Caddy: a Machine
//! that joins inside that window would look, find no Caddy to copy, and stop.

use std::io;

use ployz_core::{
    ContainerKind, ContainerObservation, CreateVolumeRequest, Machine, MachineId, ProjectName,
    ResolvedServiceSpec, ServiceMode, ServiceVolume, ServiceVolumeGraph, VolumeSource,
    derive_services,
};
use tokio_util::sync::CancellationToken;

use crate::corrosion::ReplicatedStore;
use crate::docker::ContainerRuntime;

/// Start Containers for Global Services this Machine is missing, as they appear.
///
/// # Errors
///
/// Returns when the Container subscription fails.
pub async fn run(
    machine: Machine,
    replicated: ReplicatedStore,
    containers: ContainerRuntime,
    shutdown: CancellationToken,
) -> io::Result<()> {
    let mut container_changes = replicated
        .subscribe_container_changes()
        .await
        .map_err(io::Error::other)?;
    loop {
        match replicated.containers().await {
            Ok(observed) => adopt(&machine, &containers, observed.observations).await,
            Err(error) => eprintln!("failed to read Cluster Containers: {error}"),
        }
        tokio::select! {
            changed = container_changes.changed() => changed.map_err(io::Error::other)?,
            () = shutdown.cancelled() => return Ok(()),
        }
    }
}

/// Best effort: one Service failing leaves the rest adoptable.
async fn adopt(
    machine: &Machine,
    containers: &ContainerRuntime,
    observations: Vec<ContainerObservation>,
) {
    for missing in missing_here(observations, &machine.id) {
        let service = missing.spec.name.as_str().to_owned();
        if let Err(error) = start(&machine.id, machine.subnet.gateway(), containers, &missing).await
        {
            eprintln!("failed to run Global Service {service}: {error}");
        } else {
            tracing::info!(service, "started Global Service");
        }
    }
}

pub(crate) async fn start(
    machine_id: &MachineId,
    gateway: ployz_core::MachineGateway,
    containers: &ContainerRuntime,
    missing: &MissingGlobal,
) -> Result<(), crate::docker::Error> {
    // `create` inspects named Volumes but never creates them, and a Global
    // Service owns its Volumes per Machine, so this Machine makes its own.
    for volume in named_volumes(&missing.spec.volume_graph) {
        containers.create_volume(machine_id, volume).await?;
    }
    let created = containers
        .create(
            machine_id,
            gateway,
            ContainerKind::ServiceContainer,
            &missing.project,
            &missing.spec,
        )
        .await?;
    // Creating without starting leaves a Container that observes as present,
    // which would make the Cluster look converged while serving nothing.
    containers.start(&created.container_id).await
}

fn named_volumes(graph: &ServiceVolumeGraph) -> Vec<CreateVolumeRequest> {
    graph.volumes().iter().filter_map(volume_request).collect()
}

fn volume_request(volume: &ServiceVolume) -> Option<CreateVolumeRequest> {
    let VolumeSource::Named {
        name,
        driver,
        external,
        ..
    } = &volume.source
    else {
        return None;
    };
    if *external {
        return None;
    }
    Some(CreateVolumeRequest {
        name: name.clone(),
        driver: driver
            .as_ref()
            .map_or_else(|| "local".to_owned(), |driver| driver.name.clone()),
        options: driver
            .as_ref()
            .map(|driver| driver.options.clone())
            .unwrap_or_default(),
        labels: std::collections::BTreeMap::new(),
    })
}

#[derive(Debug)]
pub(crate) struct MissingGlobal {
    pub(crate) project: ProjectName,
    pub(crate) spec: ResolvedServiceSpec,
}

/// Global Services with no Container on this Machine, newest spec per Service.
pub(crate) fn missing_here(
    observations: impl IntoIterator<Item = ContainerObservation>,
    local_id: &MachineId,
) -> Vec<MissingGlobal> {
    derive_services(observations)
        .into_iter()
        .filter_map(|service| {
            let newest = service.containers.iter().max_by_key(|container| {
                let observation = container.as_observation();
                (
                    observation.created_at_unix_nanos,
                    observation.container_id.as_str(),
                )
            })?;
            let observation = newest.as_observation();
            if observation.resolved_spec.mode != ServiceMode::Global {
                return None;
            }
            if service
                .containers
                .iter()
                .any(|container| container.as_observation().machine_id == *local_id)
            {
                return None;
            }
            Some(MissingGlobal {
                project: observation.project_name.clone(),
                spec: observation.resolved_spec.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ployz_core::{ContainerId, ContainerRuntimeObservation, ServiceId, ServiceName};
    use serde_json::json;

    use super::*;

    fn machine_id(byte: char) -> MachineId {
        MachineId::parse(std::iter::repeat_n(byte, 32).collect::<String>()).unwrap()
    }

    fn observation(
        service: &str,
        mode: serde_json::Value,
        on: &MachineId,
        container: char,
        created_at_unix_nanos: i64,
    ) -> ContainerObservation {
        let service_id = ServiceId::parse("a".repeat(32)).unwrap();
        let service_name = ServiceName::parse(service).unwrap();
        let resolved_spec: ResolvedServiceSpec = serde_json::from_value(json!({
            "service_id": service_id,
            "name": service_name,
            "mode": mode,
            "container": { "image": "caddy", "pull_policy": "missing" }
        }))
        .unwrap();
        ContainerObservation {
            container_id: ContainerId::parse(
                std::iter::repeat_n(container, 64).collect::<String>(),
            )
            .unwrap(),
            display_name: format!("{service}-c"),
            created_at_unix_nanos,
            machine_id: *on,
            project_name: ProjectName::parse("ployz-system").unwrap(),
            service_id,
            service_name,
            kind: ContainerKind::ServiceContainer,
            runtime: ContainerRuntimeObservation::Created,
            effective_healthcheck: None,
            resolved_spec,
            address: None,
            labels: BTreeMap::new(),
        }
    }

    fn global() -> serde_json::Value {
        json!({ "mode": "global" })
    }

    #[test]
    fn a_global_service_running_only_elsewhere_is_missing_here() {
        let founder = machine_id('a');
        let joiner = machine_id('b');
        let missing = missing_here([observation("caddy", global(), &founder, 'c', 0)], &joiner);

        let [only] = missing.as_slice() else {
            panic!("Caddy should be missing on the joiner");
        };
        assert_eq!(only.spec.name.as_str(), "caddy");
        assert_eq!(only.project.as_str(), "ployz-system");
    }

    #[test]
    fn nothing_is_missing_once_this_machine_holds_one() {
        let founder = machine_id('a');
        let joiner = machine_id('b');
        let missing = missing_here(
            [
                observation("caddy", global(), &founder, 'c', 0),
                observation("caddy", global(), &joiner, 'd', 1),
            ],
            &joiner,
        );

        assert!(missing.is_empty(), "already running here");
    }

    #[test]
    fn replicated_placement_stays_the_deployers() {
        let founder = machine_id('a');
        let joiner = machine_id('b');
        let replicated = json!({ "mode": "replicated", "replicas": 1 });
        let missing = missing_here([observation("api", replicated, &founder, 'c', 0)], &joiner);

        assert!(missing.is_empty(), "only Global placement is per Machine");
    }

    #[test]
    fn the_newest_spec_wins_when_a_service_was_redeployed() {
        let founder = machine_id('a');
        let joiner = machine_id('b');
        let mut older = observation("caddy", global(), &founder, 'c', 1);
        older.resolved_spec.container.image = "caddy:old".into();
        let mut newer = observation("caddy", global(), &founder, 'd', 2);
        newer.resolved_spec.container.image = "caddy:new".into();

        let missing = missing_here([older, newer], &joiner);

        let [only] = missing.as_slice() else {
            panic!("expected one missing Caddy");
        };
        assert_eq!(only.spec.container.image, "caddy:new");
    }

    #[test]
    fn bind_volumes_need_no_creation_but_named_ones_do() {
        // Caddy binds, so it must ask for nothing; `create` would otherwise
        // fail on a named Volume it only ever inspects.
        let caddy = ployz::caddy::service_spec("caddy:2.10.0".into(), Vec::new(), None);
        assert!(
            named_volumes(&caddy.volume_graph).is_empty(),
            "Caddy binds; a named Volume here would fail create's inspect"
        );
    }
}
