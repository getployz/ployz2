//! Machine-local, add-only convergence for Global Service slots.

use std::{
    io,
    time::{Duration, SystemTime},
};

use chrono::{DateTime, SecondsFormat, Utc};
use ployz_core::{
    ContainerObservation, ContainerRuntimeObservation, GlobalReconcileFailureObservation, Machine,
    QualifiedService, ResolvedServiceSpec, ServiceContainer, ServiceMode, ServiceObservation,
    derive_services, machine_matches_placement,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{corrosion::ReplicatedStore, rpc::MachineService};

const RECONCILE_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug, PartialEq)]
struct GlobalReconcileSlot {
    identity: QualifiedService,
    spec: ResolvedServiceSpec,
}

#[derive(Clone)]
pub(crate) struct GlobalReconcileObservations {
    failures: watch::Sender<Vec<GlobalReconcileFailureObservation>>,
}

impl GlobalReconcileObservations {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            failures: watch::channel(Vec::new()).0,
        }
    }

    #[must_use]
    pub(crate) fn failures(&self) -> Vec<GlobalReconcileFailureObservation> {
        self.failures.borrow().clone()
    }

    fn replace(&self, failures: Vec<GlobalReconcileFailureObservation>) {
        self.failures.send_replace(failures);
    }
}

trait GlobalSlotEnsurer {
    async fn ensure_global_slot(&self, slot: &GlobalReconcileSlot) -> Result<(), String>;
}

impl GlobalSlotEnsurer for MachineService {
    async fn ensure_global_slot(&self, slot: &GlobalReconcileSlot) -> Result<(), String> {
        self.ensure_local_global_slot(&slot.identity, &slot.spec)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

pub(crate) async fn run(
    store: ReplicatedStore,
    machine: Machine,
    ensurer: MachineService,
    observations: GlobalReconcileObservations,
    mut participating: watch::Receiver<bool>,
    shutdown: CancellationToken,
) -> io::Result<()> {
    if *participating.borrow() {
        reconcile_store(&store, &machine, &ensurer, &observations).await;
    }
    let mut interval = tokio::time::interval_at(
        tokio::time::Instant::now() + RECONCILE_INTERVAL,
        RECONCILE_INTERVAL,
    );
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let reconcile = tokio::select! {
            biased;
            () = shutdown.cancelled() => return Ok(()),
            changed = participating.changed() => match changed {
                Ok(()) => *participating.borrow(),
                Err(_) if shutdown.is_cancelled() => return Ok(()),
                Err(error) => return Err(io::Error::other(error)),
            },
            _ = interval.tick() => *participating.borrow(),
        };
        if reconcile {
            reconcile_store(&store, &machine, &ensurer, &observations).await;
        }
    }
}

async fn reconcile_store<E: GlobalSlotEnsurer>(
    store: &ReplicatedStore,
    machine: &Machine,
    ensurer: &E,
    observations: &GlobalReconcileObservations,
) {
    match store.containers().await {
        Ok(snapshot) => {
            reconcile_and_publish(
                &snapshot.observations,
                machine,
                ensurer,
                observations,
                &rfc3339(SystemTime::now()),
            )
            .await;
        }
        Err(error) => eprintln!("failed to read Globals for local reconciliation: {error}"),
    }
}

fn rfc3339(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Secs, true)
}

async fn ensure_missing_global_slots<E: GlobalSlotEnsurer>(
    containers: &[ContainerObservation],
    machine: &Machine,
    ensurer: &E,
    observed_at: &str,
) -> Vec<GlobalReconcileFailureObservation> {
    let mut failures = Vec::new();
    for slot in missing_global_slots(containers, machine) {
        if let Err(last_error) = ensurer.ensure_global_slot(&slot).await {
            failures.push(GlobalReconcileFailureObservation {
                service: slot.identity,
                last_error,
                observed_at: observed_at.into(),
            });
        }
    }
    failures
}

async fn reconcile_and_publish<E: GlobalSlotEnsurer>(
    containers: &[ContainerObservation],
    machine: &Machine,
    ensurer: &E,
    observations: &GlobalReconcileObservations,
    observed_at: &str,
) {
    observations
        .replace(ensure_missing_global_slots(containers, machine, ensurer, observed_at).await);
}

fn missing_global_slots(
    containers: &[ContainerObservation],
    machine: &Machine,
) -> Vec<GlobalReconcileSlot> {
    derive_services(containers.iter().cloned())
        .iter()
        .filter_map(|service| missing_global_slot(service, machine))
        .collect()
}

fn missing_global_slot(
    service: &ServiceObservation,
    machine: &Machine,
) -> Option<GlobalReconcileSlot> {
    let newest = newest_service_container(service)?;
    let spec = &newest.as_observation().resolved_spec;
    if spec.mode != ServiceMode::Global
        || !machine_matches_placement(machine, &spec.placement)
        || has_running_slot(&service.containers, machine, &spec.service_id)
    {
        return None;
    }
    Some(GlobalReconcileSlot {
        identity: service.identity.clone(),
        spec: spec.clone(),
    })
}

fn newest_service_container(service: &ServiceObservation) -> Option<&ServiceContainer> {
    service.containers.iter().max_by_key(|container| {
        let observation = container.as_observation();
        (
            observation.created_at_unix_nanos,
            observation.container_id.as_str(),
        )
    })
}

fn has_running_slot(
    containers: &[ServiceContainer],
    machine: &Machine,
    service_id: &ployz_core::ServiceId,
) -> bool {
    containers.iter().any(|container| {
        let observation = container.as_observation();
        observation.machine_id == machine.id
            && observation.service_id == *service_id
            && matches!(
                observation.runtime,
                ContainerRuntimeObservation::Running { .. }
            )
    })
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv6Addr, num::NonZeroU32, sync::Mutex};

    use ployz_core::{
        ContainerId, ContainerKind, ContainerRuntimeObservation, HealthObservation, MachineId,
        MachineName, MachineTarget, ManagementAddress, Placement, ProjectName, ServiceId,
        ServiceMode, ServiceName, WireGuardPublicKey,
    };
    use serde_json::json;

    use super::*;

    #[test]
    fn decision_selects_only_missing_eligible_globals_including_system_services() {
        let local = machine('1', "local");
        let peer = machine('2', "peer");
        let mut local_present = observation(
            &local,
            'e',
            "app",
            "present",
            ServiceMode::Global,
            Placement::default(),
        );
        local_present.container_id = ContainerId::parse("f".repeat(64)).unwrap();
        let containers = [
            observation(
                &peer,
                'a',
                "app",
                "api",
                ServiceMode::Global,
                Placement::default(),
            ),
            observation(
                &peer,
                'c',
                "ployz-system",
                "caddy",
                ServiceMode::Global,
                Placement::default(),
            ),
            observation(
                &peer,
                'b',
                "app",
                "worker",
                ServiceMode::Replicated {
                    replicas: NonZeroU32::new(1).unwrap(),
                },
                Placement::default(),
            ),
            observation(
                &peer,
                'd',
                "app",
                "scoped",
                ServiceMode::Global,
                Placement {
                    machines: vec![MachineTarget::parse("peer").unwrap()],
                },
            ),
            observation(
                &peer,
                'e',
                "app",
                "present",
                ServiceMode::Global,
                Placement::default(),
            ),
            local_present,
        ];

        let identities = missing_global_slots(&containers, &local)
            .into_iter()
            .map(|slot| slot.identity.to_string())
            .collect::<Vec<_>>();

        assert_eq!(identities, ["app/api", "ployz-system/caddy"]);
    }

    #[tokio::test]
    async fn effect_ensures_each_selected_slot_and_reports_only_failures() {
        let local = machine('1', "local");
        let peer = machine('2', "peer");
        let containers = [
            observation(
                &peer,
                'a',
                "app",
                "api",
                ServiceMode::Global,
                Placement::default(),
            ),
            observation(
                &peer,
                'c',
                "ployz-system",
                "caddy",
                ServiceMode::Global,
                Placement::default(),
            ),
        ];
        let ensurer = FakeEnsurer {
            calls: Mutex::new(Vec::new()),
            failing: Mutex::new(Some("ployz-system/caddy")),
        };

        let failures =
            ensure_missing_global_slots(&containers, &local, &ensurer, "2026-08-24T20:00:00Z")
                .await;

        let mut calls = ensurer.calls.lock().unwrap().clone();
        calls.sort();
        assert_eq!(calls, ["app/api", "ployz-system/caddy"]);
        assert_eq!(
            failures,
            [ployz_core::GlobalReconcileFailureObservation {
                service: QualifiedService::system_caddy(),
                last_error: "pull failed".into(),
                observed_at: "2026-08-24T20:00:00Z".into(),
            }]
        );
    }

    #[tokio::test]
    async fn successful_pass_clears_the_previous_failure_observation() {
        let local = machine('1', "local");
        let peer = machine('2', "peer");
        let containers = [observation(
            &peer,
            'a',
            "app",
            "api",
            ServiceMode::Global,
            Placement::default(),
        )];
        let ensurer = FakeEnsurer {
            calls: Mutex::new(Vec::new()),
            failing: Mutex::new(Some("app/api")),
        };
        let observations = GlobalReconcileObservations::new();

        reconcile_and_publish(
            &containers,
            &local,
            &ensurer,
            &observations,
            "2026-08-24T20:00:00Z",
        )
        .await;
        assert_eq!(observations.failures().len(), 1);

        *ensurer.failing.lock().unwrap() = None;
        reconcile_and_publish(
            &containers,
            &local,
            &ensurer,
            &observations,
            "2026-08-24T20:05:00Z",
        )
        .await;
        assert!(observations.failures().is_empty());
    }

    struct FakeEnsurer {
        calls: Mutex<Vec<String>>,
        failing: Mutex<Option<&'static str>>,
    }

    impl GlobalSlotEnsurer for FakeEnsurer {
        async fn ensure_global_slot(&self, slot: &GlobalReconcileSlot) -> Result<(), String> {
            let identity = slot.identity.to_string();
            self.calls.lock().unwrap().push(identity.clone());
            if self.failing.lock().unwrap().as_ref() == Some(&identity.as_str()) {
                Err("pull failed".into())
            } else {
                Ok(())
            }
        }
    }

    fn machine(id: char, name: &str) -> Machine {
        Machine {
            id: MachineId::parse(id.to_string().repeat(32)).unwrap(),
            name: MachineName::parse(name).unwrap(),
            subnet: format!("10.210.{}.0/24", id.to_digit(16).unwrap())
                .parse()
                .unwrap(),
            management_address: ManagementAddress(Ipv6Addr::LOCALHOST),
            public_key: WireGuardPublicKey([id as u8; 32]),
            public_ip: None,
            advertised_endpoints: Vec::new(),
            runtime: Default::default(),
        }
    }

    fn observation(
        machine: &Machine,
        id: char,
        project: &str,
        name: &str,
        mode: ServiceMode,
        placement: Placement,
    ) -> ContainerObservation {
        let service_id = ServiceId::parse(id.to_string().repeat(32)).unwrap();
        let service_name = ServiceName::parse(name).unwrap();
        let mode = serde_json::to_value(mode).unwrap();
        let mut spec: ResolvedServiceSpec = serde_json::from_value(json!({
            "service_id": service_id,
            "name": service_name,
            "mode": mode,
            "container": { "image": "alpine:3.23.3", "pull_policy": "missing" }
        }))
        .unwrap();
        spec.placement = placement;
        ContainerObservation {
            container_id: ContainerId::parse(id.to_string().repeat(64)).unwrap(),
            display_name: format!("{name}-{id}"),
            created_at_unix_nanos: 1,
            machine_id: machine.id,
            project_name: ProjectName::parse(project).unwrap(),
            service_id,
            service_name,
            kind: ContainerKind::ServiceContainer,
            runtime: ContainerRuntimeObservation::Running {
                health: HealthObservation::Healthy,
            },
            effective_healthcheck: None,
            resolved_spec: spec,
            address: None,
            labels: Default::default(),
        }
    }
}
