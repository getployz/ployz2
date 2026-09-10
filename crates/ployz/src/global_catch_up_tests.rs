//! Tests for bounded Global catch-up and target storage eligibility.

use std::{cell::Cell, num::NonZeroU64};

use ployz_core::{
    ContainerId, ContainerKind, ContainerObservation, ContainerPath, ContainerResources,
    ContainerRuntimeObservation, DockerVolumeName, HealthObservation, Machine, MachineId,
    MachineName, MachineStorageObservation, Placement, ProjectName, ProvisionedVolumeMaximumBytes,
    PullPolicy, RequestedServiceSpec, ResolvedServiceSpec, ResolvedUpdateConfig, RestartPolicy,
    ServiceContainerSpec, ServiceId, ServiceMode, ServiceMount, ServiceName, ServiceObservation,
    ServiceVolume, ServiceVolumeGraph, ServiceVolumeReference, UpdateConfig, WireGuardPublicKey,
    service_containers,
};

use super::*;

#[tokio::test]
async fn partial_observations_reject_catch_up_before_any_placement() {
    let joiner = machine('1', "joiner");
    let peer = machine('f', "peer");
    for failed in [true, false] {
        let mut client = FakeCatchUpClient {
            machine_id: joiner.id,
            services: Vec::new(),
            target_services: None,
            capacity: None,
            storage: Ok(None),
            create_error: None,
            ensure_calls: Cell::new(0),
            failures: if failed {
                vec![ployz_core::MachineFailure {
                    machine_id: peer.id,
                    error: RpcError {
                        code: ployz_core::RpcErrorCode::Unavailable,
                        message: "peer unavailable".into(),
                        details: serde_json::Value::Null,
                    },
                }]
            } else {
                Vec::new()
            },
            omissions: if failed { Vec::new() } else { vec![peer.id] },
        };
        let error = catch_up_globals(&mut client, &joiner).await.unwrap_err();
        let message = joined_catch_up_error(error);
        assert!(
            message.contains("partial Service observations"),
            "{message}"
        );
        assert!(message.contains(peer.id.as_str()), "{message}");
        assert_eq!(client.ensure_calls.get(), 0);
    }
}

#[tokio::test]
async fn stale_local_generation_checks_capacity_before_ensuring_current_slot() {
    let joiner = machine('1', "joiner");
    let founder = machine('f', "founder");
    let stale = global_service(
        qualified("app", "api"),
        'a',
        Placement::default(),
        created_on(&joiner, 'a'),
    );
    let current = global_service_with_image(
        qualified("app", "api"),
        'b',
        Placement::default(),
        running_on(&founder, 'b'),
        "ghcr.io/getployz/api:2",
    );
    let mut client = FakeCatchUpClient {
        machine_id: joiner.id,
        services: vec![stale, current],
        target_services: None,
        capacity: None,
        create_error: None,
        ensure_calls: Cell::new(0),
        failures: Vec::new(),
        omissions: Vec::new(),
        storage: Ok(None),
    };

    assert!(catch_up_globals(&mut client, &joiner).await.is_err());
    assert_eq!(client.ensure_calls.get(), 0);
}

#[tokio::test]
async fn successful_ensure_is_reobserved_before_success() {
    let joiner = machine('1', "joiner");
    let service = global_service(
        qualified("app", "api"),
        'a',
        Placement::default(),
        created_on(&joiner, 'a'),
    );
    let mut client = FakeCatchUpClient {
        machine_id: joiner.id,
        services: vec![service],
        target_services: None,
        capacity: None,
        create_error: None,
        ensure_calls: Cell::new(0),
        failures: Vec::new(),
        omissions: Vec::new(),
        storage: Ok(None),
    };

    let error = catch_up_globals(&mut client, &joiner).await.unwrap_err();
    assert_eq!(client.ensure_calls.get(), 1);
    assert_eq!(error.unresolved, [qualified("app", "api")]);
}

#[tokio::test]
async fn initially_eligible_global_absent_from_target_inspection_remains_missing() {
    let joiner = machine('1', "joiner");
    let founder = machine('f', "founder");
    let service = global_service(
        qualified("app", "api"),
        'a',
        Placement::default(),
        running_on(&founder, 'a'),
    );
    let mut client = FakeCatchUpClient {
        machine_id: joiner.id,
        services: vec![service],
        target_services: Some(Vec::new()),
        capacity: None,
        create_error: None,
        ensure_calls: Cell::new(0),
        failures: Vec::new(),
        omissions: Vec::new(),
        storage: Ok(None),
    };

    let error = catch_up_globals(&mut client, &joiner).await.unwrap_err();
    assert_eq!(error.unresolved, [qualified("app", "api")]);
}

#[tokio::test]
async fn initially_eligible_global_with_only_hook_visible_remains_missing() {
    let joiner = machine('1', "joiner");
    let founder = machine('f', "founder");
    let service = global_service(
        qualified("app", "api"),
        'a',
        Placement::default(),
        running_on(&founder, 'a'),
    );
    let mut hook = service
        .containers
        .first()
        .expect("test Global has one Service container")
        .clone()
        .into_observation();
    hook.try_update(|parts| parts.kind = ContainerKind::PreDeployHook)
        .unwrap();
    let hook_only = ServiceObservation {
        identity: service.identity.clone(),
        service_id: service.service_id,
        containers: Vec::new(),
        hook_containers: vec![ployz_core::HookContainer::try_from(hook).unwrap()],
    };
    let mut client = FakeCatchUpClient {
        machine_id: joiner.id,
        services: vec![service],
        target_services: Some(vec![hook_only]),
        capacity: None,
        create_error: None,
        ensure_calls: Cell::new(0),
        failures: Vec::new(),
        omissions: Vec::new(),
        storage: Ok(None),
    };

    let error = catch_up_globals(&mut client, &joiner).await.unwrap_err();
    assert_eq!(error.unresolved, [qualified("app", "api")]);
}

#[tokio::test]
async fn initially_eligible_generation_absent_from_target_inspection_remains_missing() {
    let joiner = machine('1', "joiner");
    let founder = machine('f', "founder");
    let stale = global_service(
        qualified("app", "api"),
        'a',
        Placement::default(),
        running_on(&joiner, 'a'),
    );
    let current = global_service_with_image(
        qualified("app", "api"),
        'b',
        Placement::default(),
        running_on(&founder, 'b'),
        "ghcr.io/getployz/api:2",
    );
    let mut client = FakeCatchUpClient {
        machine_id: joiner.id,
        services: vec![stale.clone(), current],
        target_services: Some(vec![stale]),
        capacity: Some(BridgeEndpointCapacity::new(10, 0)),
        create_error: None,
        ensure_calls: Cell::new(0),
        failures: Vec::new(),
        omissions: Vec::new(),
        storage: Ok(None),
    };

    let error = catch_up_globals(&mut client, &joiner).await.unwrap_err();
    assert_eq!(client.ensure_calls.get(), 1);
    assert_eq!(error.unresolved, [qualified("app", "api")]);
}

#[tokio::test]
async fn another_projects_matching_shape_does_not_satisfy_catch_up() {
    let joiner = machine('1', "joiner");
    let founder = machine('f', "founder");
    let shop = global_service(
        qualified("shop", "api"),
        'c',
        Placement::default(),
        running_on(&joiner, 'c'),
    );
    let prod = global_service(
        qualified("prod", "api"),
        'd',
        Placement::default(),
        running_on(&founder, 'e'),
    );
    let mut client = FakeCatchUpClient {
        machine_id: joiner.id,
        services: vec![shop.clone(), prod],
        target_services: Some(vec![shop]),
        capacity: Some(BridgeEndpointCapacity::new(10, 0)),
        create_error: None,
        ensure_calls: Cell::new(0),
        failures: Vec::new(),
        omissions: Vec::new(),
        storage: Ok(None),
    };

    let error = catch_up_globals(&mut client, &joiner).await.unwrap_err();
    assert_eq!(client.ensure_calls.get(), 1);
    assert_eq!(error.unresolved, [qualified("prod", "api")]);
}

struct FakeCatchUpClient {
    machine_id: MachineId,
    services: Vec<ServiceObservation>,
    target_services: Option<Vec<ServiceObservation>>,
    capacity: Option<BridgeEndpointCapacity>,
    storage: Result<Option<MachineStorageObservation>, &'static str>,
    ensure_calls: Cell<usize>,
    create_error: Option<&'static str>,
    failures: Vec<ployz_core::MachineFailure<RpcError>>,
    omissions: Vec<MachineId>,
}

impl CatchUpClient for FakeCatchUpClient {
    async fn live_services(&mut self) -> Result<LiveServices<RpcError>, Failure> {
        Ok(LiveServices {
            containers: ployz_core::PartialResult {
                successes: vec![ployz_core::MachineSuccess {
                    machine_id: self.machine_id,
                    value: self
                        .services
                        .iter()
                        .flat_map(ServiceObservation::members)
                        .map(|container| container.as_observation().clone())
                        .collect(),
                }],
                failures: self.failures.clone(),
                omissions: self.omissions.clone(),
            },
        })
    }

    async fn bridge_capacity(
        &mut self,
        _machine_id: &MachineId,
    ) -> Result<Option<BridgeEndpointCapacity>, Failure> {
        Ok(self.capacity.clone())
    }

    async fn create_slot(
        &mut self,
        _machine_id: &MachineId,
        request: CreateContainerRequest,
    ) -> Result<Option<ployz_core::ContainerCreated>, RpcError> {
        let target = machine('1', "joiner");
        let storage = self.storage.ok().flatten();
        let eligibility = request.resolved_spec.placement_eligibility_in_project(
            &request.project_name,
            &target,
            storage.as_ref(),
        );
        match eligibility {
            ServicePlacementEligibility::Ineligible(_) => return Ok(None),
            ServicePlacementEligibility::Unknown(_) => {
                return Err(RpcError {
                    code: ployz_core::RpcErrorCode::Conflict,
                    message: format!(
                        "storage eligibility is unknown: {}",
                        self.storage.err().unwrap_or("missing storage evidence")
                    ),
                    details: serde_json::Value::Null,
                });
            }
            ServicePlacementEligibility::Eligible => {}
        }
        let existing = self
            .services
            .iter()
            .flat_map(|service| &service.containers)
            .find(|container| {
                let c = container.as_observation();
                c.machine_id == target.id
                    && c.project_name == request.project_name
                    && c.resolved_spec == request.resolved_spec
            });
        if existing.is_none()
            && let Some(error) = endpoint_capacity_error(1, self.capacity.as_ref())
        {
            return Err(RpcError {
                code: ployz_core::RpcErrorCode::Conflict,
                message: error.to_string(),
                details: serde_json::Value::Null,
            });
        }
        if !existing.is_some_and(|container| {
            matches!(
                container.as_observation().runtime,
                ContainerRuntimeObservation::Running { .. }
            )
        }) {
            self.ensure_calls.set(self.ensure_calls.get() + 1);
        }
        if let Some(message) = self.create_error {
            return Err(RpcError {
                code: ployz_core::RpcErrorCode::Conflict,
                message: message.into(),
                details: serde_json::Value::Null,
            });
        }
        Ok(Some(ployz_core::ContainerCreated {
            container_id: container_id('a'),
            display_name: "api".into(),
        }))
    }

    async fn start_slot(
        &mut self,
        _machine_id: &MachineId,
        _container_id: ContainerId,
    ) -> Result<(), RpcError> {
        Ok(())
    }

    async fn target_containers(
        &mut self,
        _machine_id: &MachineId,
    ) -> Result<Vec<ContainerObservation>, Failure> {
        Ok(self
            .target_services
            .as_ref()
            .unwrap_or(&self.services)
            .iter()
            .flat_map(ServiceObservation::members)
            .map(|container| container.as_observation().clone())
            .collect())
    }
}

#[tokio::test]
async fn provisioned_globals_use_target_storage_and_report_unknown() {
    let joiner = machine('1', "joiner");
    let founder = machine('f', "founder");
    let spec = provisioned_global_spec();
    let service = grouped(
        qualified("app", "api"),
        spec.to_resolved(service_id('a'), ResolvedUpdateConfig::default())
            .expect("volume graph is scoped"),
        running_on(&founder, 'a'),
    );

    for (storage, expected_calls, incomplete) in [
        (Ok(Some(MachineStorageObservation::Ready)), 1, false),
        (
            Ok(Some(MachineStorageObservation::Pool {
                size_bytes: NonZeroU64::new(100).unwrap(),
                used_bytes: 0,
                free_bytes: 100,
            })),
            1,
            false,
        ),
        (Ok(Some(MachineStorageObservation::Stateless)), 0, false),
        (Ok(None), 0, true),
        (Err("storage inspection failed"), 0, true),
    ] {
        let local = grouped(
            service.identity.clone(),
            service.observed_global_slot_spec().unwrap().clone(),
            running_on(&joiner, 'b'),
        );
        let stateless = global_service(
            qualified("app", "worker"),
            'c',
            Placement::default(),
            running_on(&founder, 'c'),
        );
        let local_stateless = global_service(
            qualified("app", "worker"),
            'c',
            Placement::default(),
            running_on(&joiner, 'd'),
        );
        let mut client = FakeCatchUpClient {
            machine_id: joiner.id,
            services: vec![service.clone(), stateless],
            target_services: Some(vec![local, local_stateless]),
            capacity: Some(BridgeEndpointCapacity::new(10, 0)),
            storage,
            create_error: None,
            ensure_calls: Cell::new(0),
            failures: Vec::new(),
            omissions: Vec::new(),
        };
        let result = catch_up_globals(&mut client, &joiner).await;
        assert_eq!(client.ensure_calls.get(), expected_calls + 1);
        assert_eq!(result.is_err(), incomplete);
        if let Err(error) = result {
            assert_eq!(error.unresolved, [qualified("app", "api")]);
            let message = joined_catch_up_error(error);
            assert!(message.contains("storage eligibility is unknown"));
            if let Err(cause) = storage {
                assert!(message.contains(cause));
            }
        }
    }
}

fn machine(hex: char, name: &str) -> Machine {
    Machine {
        labels: Default::default(),
        accepts_builds: true,
        accepts_services: true,
        accepts_ingress: true,
        id: MachineId::parse(hex.to_string().repeat(32)).unwrap(),
        name: MachineName::parse(name).unwrap(),
        subnet: format!("10.210.{}.0/24", hex.to_digit(16).unwrap())
            .parse()
            .unwrap(),
        public_key: WireGuardPublicKey([hex as u8; 32]),
        public_ip: None,
        advertised_endpoints: Vec::new(),
        runtime: Default::default(),
    }
}

fn qualified(project: &str, name: &str) -> QualifiedService {
    QualifiedService::new(
        ProjectName::parse(project).unwrap(),
        ServiceName::parse(name).unwrap(),
    )
}

fn service_id(hex: char) -> ServiceId {
    ServiceId::parse(hex.to_string().repeat(32)).unwrap()
}

fn container_id(hex: char) -> ContainerId {
    ContainerId::parse(hex.to_string().repeat(64)).unwrap()
}

fn requested(mode: ServiceMode) -> RequestedServiceSpec {
    RequestedServiceSpec {
        name: ServiceName::parse("api").unwrap(),
        mode,
        container: ServiceContainerSpec {
            image: "ghcr.io/getployz/api:1".into(),
            command: Vec::new(),
            entrypoint: Vec::new(),
            environment: Default::default(),
            labels: Default::default(),
            hostname: None,
            extra_hosts: Vec::new(),
            cap_add: Vec::new(),
            cap_drop: Vec::new(),
            healthcheck: None,
            pull_policy: PullPolicy::Missing,
            init: None,
            user: None,
            working_directory: None,
            tty: false,
            open_stdin: false,
            privileged: false,
            pid_mode: None,
            log_driver: None,
            resources: ContainerResources::default(),
            stop_timeout_secs: None,
            sysctls: Default::default(),
            restart: RestartPolicy::default(),
        },
        placement: Placement::default(),
        ports: Vec::new(),
        mount_graph: Default::default(),
        pre_deploy: None,
        ingress_proxy_fragment: None,
        update: UpdateConfig::default(),
    }
}

fn global_service(
    identity: QualifiedService,
    id: char,
    placement: Placement,
    container: ContainerObservation,
) -> ServiceObservation {
    global_service_with_image(identity, id, placement, container, "ghcr.io/getployz/api:1")
}

fn global_service_with_image(
    identity: QualifiedService,
    id: char,
    placement: Placement,
    container: ContainerObservation,
    image: &str,
) -> ServiceObservation {
    let mut spec = requested(ServiceMode::Global);
    spec.name = identity.name.clone();
    spec.placement = placement;
    spec.container.image = image.into();
    grouped(
        identity,
        spec.to_resolved(service_id(id), ResolvedUpdateConfig::default())
            .expect("volume graph is scoped"),
        container,
    )
}

fn grouped(
    identity: QualifiedService,
    spec: ResolvedServiceSpec,
    mut container: ContainerObservation,
) -> ServiceObservation {
    container
        .try_update(|parts| {
            parts.project_name = identity.project.clone();
            parts.resolved_spec = spec.clone();
        })
        .unwrap();
    ServiceObservation {
        identity,
        service_id: spec.service_id,
        containers: service_containers([container]),
        hook_containers: Vec::new(),
    }
}

fn running_on(machine: &Machine, hex: char) -> ContainerObservation {
    container_on(
        machine,
        hex,
        ContainerRuntimeObservation::Running {
            health: HealthObservation::Healthy,
        },
    )
}

fn created_on(machine: &Machine, hex: char) -> ContainerObservation {
    container_on(machine, hex, ContainerRuntimeObservation::Created)
}

fn container_on(
    machine: &Machine,
    hex: char,
    runtime: ContainerRuntimeObservation,
) -> ContainerObservation {
    ployz_core::ContainerObservation::try_from(ployz_core::ContainerObservationParts {
        container_id: container_id(hex),
        display_name: format!("slot-{hex}"),
        created_at_unix_nanos: 0,
        machine_id: machine.id,
        project_name: ProjectName::parse("app").unwrap(),
        kind: ContainerKind::ServiceContainer,
        runtime,
        effective_healthcheck: None,
        resolved_spec: requested(ServiceMode::Global)
            .to_resolved(service_id('a'), ResolvedUpdateConfig::default())
            .expect("volume graph is scoped"),
        address: None,
        labels: Default::default(),
    })
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn real_catch_up_client_retries_readiness_and_placement_to_their_budget() {
    use ployz_core::{
        ContainerCreated, InspectTelemetry, LocalMachinePhase, MachineDetails, OpaquePayload,
        RpcRequestBody, RpcResponse,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;
    use tonic::{Request, Response, Status};

    for placement in [false, true] {
        for failures in [1, 4] {
            let target = machine('1', "joiner");
            let observed = target.clone();
            let calls = Arc::new(AtomicUsize::new(0));
            let attempts = calls.clone();
            let request = CreateContainerRequest {
                creation_key: Some("global:test".into()),
                kind: ContainerKind::ServiceContainer,
                project_name: ProjectName::parse("app").unwrap(),
                resolved_spec: requested(ServiceMode::Global)
                    .to_resolved(service_id('a'), ResolvedUpdateConfig::default())
                    .unwrap(),
            };
            let expected = request.clone();
            let (mut client, server) =
                crate::connect::test_support::rpc_client(move |rpc: Request<OpaquePayload>| {
                    let target = observed.clone();
                    let expected = expected.clone();
                    let attempts = attempts.clone();
                    async move {
                        assert_eq!(
                            rpc.metadata().get(ployz_core::ONE_TARGET_HEADER).unwrap(),
                            target.id.as_str()
                        );
                        let body = rpc.into_inner().decode_request().unwrap().body;
                        let inject_failure =
                            !placement || matches!(body, RpcRequestBody::CreateContainer(_));
                        #[expect(
                            clippy::wildcard_enum_match_arm,
                            reason = "fixture accepts only the catch-up RPCs under test"
                        )]
                        let response = match body {
                            RpcRequestBody::Inspect(inspect) => {
                                if !placement {
                                    assert_eq!(inspect.telemetry, InspectTelemetry::BridgeCapacity);
                                }
                                RpcResponse::from(MachineDetails {
                                    id: target.id,
                                    phase: LocalMachinePhase::Participating,
                                    public_key: target.public_key,
                                    advertised_endpoints: Vec::new(),
                                    machine: Some(target),
                                    store_version: Default::default(),
                                    rtts: Vec::new(),
                                    cloud_paired: false,
                                    telemetry: Some(
                                        ployz_core::TelemetryObservation::BridgeCapacity {
                                            bridge: BridgeEndpointCapacity::new(10, 0),
                                        },
                                    ),
                                    storage: None,
                                })
                            }
                            RpcRequestBody::ListContainers(_) => {
                                RpcResponse::from(ployz_core::ContainerList {
                                    containers: Vec::new(),
                                })
                            }
                            RpcRequestBody::CreateContainer(ensure) => {
                                assert!(placement);
                                assert_eq!(ensure, expected);
                                RpcResponse::from(ContainerCreated {
                                    container_id: container_id('a'),
                                    display_name: "api".into(),
                                })
                            }
                            other => panic!("unexpected catch-up RPC: {other:?}"),
                        };
                        if inject_failure && attempts.fetch_add(1, Ordering::SeqCst) < failures {
                            Err(Status::unavailable("transient catch-up failure"))
                        } else {
                            Ok(Response::new(response.encode().unwrap()))
                        }
                    }
                })
                .await;
            let started = tokio::time::Instant::now();
            let succeeded = if placement {
                CatchUpClient::create_slot(&mut client, &target.id, request)
                    .await
                    .is_ok()
            } else {
                CatchUpClient::bridge_capacity(&mut client, &target.id)
                    .await
                    .is_ok()
            };
            assert_eq!(succeeded, failures == 1);
            assert_eq!(
                calls.load(Ordering::SeqCst),
                if failures == 1 { 2 } else { 4 }
            );
            assert_eq!(
                started.elapsed(),
                if failures == 1 {
                    Duration::from_millis(500)
                } else {
                    Duration::from_secs(6)
                }
            );
            server.abort();
        }
    }
}

fn provisioned_global_spec() -> RequestedServiceSpec {
    let mut spec = requested(ServiceMode::Global);
    let reference = ServiceVolumeReference::parse("data").unwrap();
    spec.set_volume_graph(
        ServiceVolumeGraph::parse(
            vec![ServiceVolume {
                reference: reference.clone(),
                source: ployz_core::RawVolumeSource::Provisioned {
                    name: DockerVolumeName::parse("data").unwrap(),
                    maximum_bytes: ProvisionedVolumeMaximumBytes::new(
                        NonZeroU64::new(100).unwrap(),
                    ),
                    labels: Default::default(),
                }
                .admit()
                .expect("valid volume declaration"),
            }],
            vec![ServiceMount {
                volume: reference,
                target: ContainerPath::parse("/data").unwrap(),
                read_only: false,
                no_copy: false,
                subpath: None,
            }],
        )
        .unwrap()
        .scope_to_project(&ployz_core::ProjectName::parse("app").unwrap())
        .unwrap(),
    )
    .unwrap();
    spec
}

#[tokio::test]
async fn failed_placement_is_reported_even_if_final_observation_is_running() {
    let joiner = machine('1', "joiner");
    let founder = machine('f', "founder");
    let mut client = FakeCatchUpClient {
        machine_id: joiner.id,
        services: vec![global_service(
            qualified("app", "api"),
            'a',
            Placement::default(),
            running_on(&founder, 'a'),
        )],
        target_services: Some(vec![global_service(
            qualified("app", "api"),
            'a',
            Placement::default(),
            running_on(&joiner, 'b'),
        )]),
        capacity: Some(BridgeEndpointCapacity::new(10, 0)),
        storage: Ok(None),
        ensure_calls: Cell::new(0),
        create_error: Some("creation key conflict"),
        failures: Vec::new(),
        omissions: Vec::new(),
    };
    let error = catch_up_globals(&mut client, &joiner).await.unwrap_err();
    assert!(joined_catch_up_error(error).contains("creation key conflict"));
}

#[tokio::test]
async fn catch_up_uses_primitives_and_never_replaces_a_key_conflict_or_unknown_slot() {
    use ployz_core::{
        ContainerChanged, ContainerList, LocalMachinePhase, MachineDetails, OpaquePayload,
        RpcRequestBody, RpcResponse,
    };
    use std::sync::{Arc, Mutex};
    use tonic::{Request, Response};

    for outcome in [
        "new",
        "matching",
        "conflict",
        "revision",
        "ineligible",
        "unknown",
    ] {
        let mut target = machine('1', "joiner");
        target.accepts_services = outcome != "ineligible";
        let spec = if outcome == "unknown" {
            provisioned_global_spec()
        } else {
            requested(ServiceMode::Global)
        }
        .to_resolved(service_id('a'), ResolvedUpdateConfig::default())
        .unwrap();
        let request = CreateContainerRequest {
            creation_key: Some(crate::cluster::global_creation_key(&spec)),
            kind: ContainerKind::ServiceContainer,
            project_name: ProjectName::parse("app").unwrap(),
            resolved_spec: spec.clone(),
        };
        let expected = request.clone();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let observed = target.clone();
        let (mut client, server) =
            crate::connect::test_support::rpc_client(move |rpc: Request<OpaquePayload>| {
                let target = observed.clone();
                let recorded = recorded.clone();
                let expected = expected.clone();
                async move {
                    assert_eq!(
                        rpc.metadata().get(ployz_core::ONE_TARGET_HEADER).unwrap(),
                        target.id.as_str()
                    );
                    #[expect(
                        clippy::wildcard_enum_match_arm,
                        reason = "fixture rejects unrelated RPCs"
                    )]
                    let response = match rpc.into_inner().decode_request().unwrap().body {
                        RpcRequestBody::Inspect(inspect) => {
                            recorded.lock().unwrap().push(if inspect.include_storage {
                                "inspect"
                            } else {
                                "capacity"
                            });
                            RpcResponse::from(MachineDetails {
                                id: target.id,
                                phase: LocalMachinePhase::Participating,
                                public_key: target.public_key,
                                machine: Some(target),
                                advertised_endpoints: Vec::new(),
                                store_version: Default::default(),
                                rtts: Vec::new(),
                                cloud_paired: false,
                                telemetry: Some(ployz_core::TelemetryObservation::BridgeCapacity {
                                    bridge: BridgeEndpointCapacity::new(10, 0),
                                }),
                                storage: None,
                            })
                        }
                        RpcRequestBody::CreateContainer(create) => {
                            recorded.lock().unwrap().push("create");
                            assert_eq!(create, expected);
                            if outcome == "conflict" {
                                RpcResponse::from(RpcError {
                                    code: ployz_core::RpcErrorCode::Conflict,
                                    message: "creation key conflict".into(),
                                    details: serde_json::Value::Null,
                                })
                            } else {
                                RpcResponse::from(ployz_core::ContainerCreated {
                                    container_id: container_id(if outcome == "revision" {
                                        'b'
                                    } else {
                                        'a'
                                    }),
                                    display_name: "existing".into(),
                                })
                            }
                        }
                        RpcRequestBody::StartContainer(start) => {
                            recorded.lock().unwrap().push("start");
                            assert_eq!(
                                start.container_id,
                                container_id(if outcome == "revision" { 'b' } else { 'a' })
                            );
                            RpcResponse::from(ContainerChanged {
                                container_id: start.container_id,
                            })
                        }
                        RpcRequestBody::ListContainers(_) => {
                            recorded.lock().unwrap().push("list");
                            if outcome == "new" {
                                return Ok(Response::new(
                                    RpcResponse::from(ContainerList {
                                        containers: Vec::new(),
                                    })
                                    .encode()
                                    .unwrap(),
                                ));
                            }
                            let mut slot = grouped(
                                qualified("app", "api"),
                                expected.resolved_spec,
                                running_on(&target, 'a'),
                            )
                            .containers
                            .remove(0)
                            .into_observation();
                            if outcome == "conflict" {
                                slot.try_update(|parts| {
                                    parts.resolved_spec.update.monitor_millis = Some(500)
                                })
                                .unwrap();
                            } else if outcome == "revision" {
                                slot.try_update(|parts| {
                                    parts.resolved_spec.container.image = "api:old".into()
                                })
                                .unwrap();
                            }
                            let mut unrelated = slot.clone();
                            unrelated
                                .try_update(|parts| {
                                    parts.project_name = ProjectName::parse("other").unwrap();
                                    parts.container_id = container_id('c');
                                })
                                .unwrap();
                            RpcResponse::from(ContainerList {
                                containers: vec![slot, unrelated],
                            })
                        }
                        RpcRequestBody::StopContainer(stop) => {
                            recorded.lock().unwrap().push("stop");
                            assert_eq!(stop.container_id, container_id('a'));
                            RpcResponse::from(ContainerChanged {
                                container_id: stop.container_id,
                            })
                        }
                        RpcRequestBody::RemoveContainer(remove) => {
                            recorded.lock().unwrap().push("remove");
                            assert_eq!(remove.container_id, container_id('a'));
                            assert!(!remove.remove_volumes);
                            RpcResponse::from(ContainerChanged {
                                container_id: remove.container_id,
                            })
                        }
                        other => panic!("unexpected RPC: {other:?}"),
                    };
                    Ok(Response::new(response.encode().unwrap()))
                }
            })
            .await;
        let created = CatchUpClient::create_slot(&mut client, &target.id, request).await;
        if let Ok(Some(created)) = &created {
            CatchUpClient::start_slot(&mut client, &target.id, created.container_id)
                .await
                .unwrap();
        }
        assert_eq!(
            created.is_ok(),
            ["matching", "new", "revision", "ineligible"].contains(&outcome)
        );
        let expected: &[&str] = match outcome {
            "new" | "revision" => &["inspect", "list", "capacity", "create", "start"],
            "matching" => &["inspect", "list", "start"],
            "conflict" => &["inspect", "list", "capacity", "create"],
            "ineligible" => &["inspect", "list", "stop", "remove"],
            _ => &["inspect"],
        };
        assert_eq!(calls.lock().unwrap().as_slice(), expected);
        server.abort();
    }
}

#[tokio::test]
async fn top_level_catch_up_uses_fresh_target_eligibility_and_reuses_hidden_full_capacity_slot() {
    use ployz_core::{
        ContainerChanged, ContainerList, LocalMachinePhase, MachineDetails, MachineList,
        MachineObservation, MembershipObservation, OpaquePayload, RpcRequestBody, RpcResponse,
        TelemetryObservation,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tonic::{Request, Response};
    for eligible in [false, true] {
        let mut stale = machine('1', "joiner");
        stale.accepts_services = false;
        let mut fresh = stale.clone();
        fresh.accepts_services = eligible;
        let founder = machine('f', "founder");
        let spec = requested(ServiceMode::Global)
            .to_resolved(service_id('a'), ResolvedUpdateConfig::default())
            .unwrap();
        let peer = grouped(
            qualified("app", "api"),
            spec.clone(),
            running_on(&founder, 'a'),
        )
        .containers
        .remove(0)
        .into_observation();
        let local = grouped(qualified("app", "api"), spec, created_on(&fresh, 'b'))
            .containers
            .remove(0)
            .into_observation();
        let retained = Arc::new(Mutex::new(vec![local]));
        let current = retained.clone();
        let local_reads = Arc::new(AtomicUsize::new(0));
        let observed = fresh.clone();
        let (mut client, server) =
            crate::connect::test_support::rpc_client(move |rpc: Request<OpaquePayload>| {
                let target = observed.clone();
                let founder = founder.clone();
                let peer = peer.clone();
                let current = current.clone();
                let reads = local_reads.clone();
                let selected = rpc
                    .metadata()
                    .get(ployz_core::ONE_TARGET_HEADER)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                async move {
                    #[expect(
                        clippy::wildcard_enum_match_arm,
                        reason = "fixture rejects unrelated operations and any duplicate creation"
                    )]
                    let response = match rpc.into_inner().decode_request().unwrap().body {
                        RpcRequestBody::ListMachines(_) => RpcResponse::from(MachineList {
                            machines: vec![
                                MachineObservation::new(target.clone(), MembershipObservation::Up),
                                MachineObservation::new(founder.clone(), MembershipObservation::Up),
                            ],
                        }),
                        RpcRequestBody::ListContainers(_) => {
                            let containers = if selected.as_deref() == Some(founder.id.as_str()) {
                                vec![peer]
                            } else if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                                Vec::new()
                            } else {
                                current.lock().unwrap().clone()
                            };
                            RpcResponse::from(ContainerList { containers })
                        }
                        RpcRequestBody::Inspect(_) => RpcResponse::from(MachineDetails {
                            id: target.id,
                            public_key: target.public_key,
                            machine: Some(target),
                            phase: LocalMachinePhase::Participating,
                            advertised_endpoints: Vec::new(),
                            store_version: Default::default(),
                            rtts: Vec::new(),
                            cloud_paired: false,
                            storage: None,
                            telemetry: Some(TelemetryObservation::BridgeCapacity {
                                bridge: BridgeEndpointCapacity::new(1, 1),
                            }),
                        }),
                        RpcRequestBody::StartContainer(start) => {
                            assert!(eligible);
                            let mut current = current.lock().unwrap();
                            let slot = current.first_mut().unwrap();
                            assert_eq!(slot.container_id, start.container_id);
                            slot.try_update(|parts| {
                                parts.runtime = ContainerRuntimeObservation::Running {
                                    health: HealthObservation::Healthy,
                                }
                            })
                            .unwrap();
                            RpcResponse::from(ContainerChanged {
                                container_id: start.container_id,
                            })
                        }
                        RpcRequestBody::StopContainer(stop) => {
                            assert!(!eligible);
                            RpcResponse::from(ContainerChanged {
                                container_id: stop.container_id,
                            })
                        }
                        RpcRequestBody::RemoveContainer(remove) => {
                            assert!(!eligible);
                            assert_eq!(remove.container_id, container_id('b'));
                            current.lock().unwrap().clear();
                            RpcResponse::from(ContainerChanged {
                                container_id: remove.container_id,
                            })
                        }
                        other => {
                            panic!("unexpected RPC (existing slot needs no endpoint): {other:?}")
                        }
                    };
                    Ok(Response::new(response.encode().unwrap()))
                }
            })
            .await;
        catch_up_globals(&mut client, &stale).await.unwrap();
        assert_eq!(retained.lock().unwrap().len(), usize::from(eligible));
        server.abort();
    }
}
