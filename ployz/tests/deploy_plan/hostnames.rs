use super::support::*;
use ployz_core::{
    HttpProtocol, IngressHost, IngressHostname, PortPublication, ProjectName, QualifiedService,
};

#[test]
fn complete_snapshot_rejects_another_qualified_service_already_publishing_the_custom_hostname() {
    let spec = custom_web();
    let snapshot = snapshot_with(vec![other_project_container(&spec, 1)]);
    let error = plan_deploy([&spec], &snapshot, PlanOptions::default()).unwrap_err();
    assert_eq!(
        error,
        PlanError::CustomHostnameConflict {
            hostname: IngressHost::parse("api.example.com").unwrap(),
            owner: QualifiedService::parse("blog/web").unwrap(),
        }
    );
    assert_eq!(
        error.to_string(),
        "custom hostname api.example.com is already published by blog/web"
    );
}

#[test]
fn same_qualified_service_redeploy_keeps_the_custom_hostname() {
    let spec = custom_web();
    let existing = container('c', '1', &spec, &service_id('a'));
    let snapshot = snapshot_with(vec![existing]);
    let plan = plan_deploy([&spec], &snapshot, PlanOptions::default()).unwrap();
    assert!(!plan.observer_relative_hostname_detection);
}

#[test]
fn incomplete_snapshot_does_not_claim_uniqueness_and_warns_that_detection_is_observer_relative() {
    let spec = custom_web();
    let snapshot = DeploySnapshot {
        volume_omissions: vec![machine_id('1')],
        ..snapshot_with(vec![other_project_container(&spec, 1)])
    };
    assert!(!snapshot.is_observer_complete());
    let plan = plan_deploy([&spec], &snapshot, PlanOptions::default()).unwrap();
    assert!(plan.observer_relative_hostname_detection);
    assert!(
        plan.operations
            .iter()
            .any(|operation| { matches!(operation, DeployOperation::RunContainer { .. }) })
    );
}

#[test]
fn complete_snapshot_without_a_conflict_does_not_warn() {
    let spec = custom_web();
    let plan = plan_deploy([&spec], &snapshot_with(Vec::new()), PlanOptions::default()).unwrap();
    assert!(!plan.observer_relative_hostname_detection);
}

fn custom_web() -> RequestedServiceSpec {
    let mut spec = requested(ServiceMode::Replicated {
        replicas: NonZeroU32::new(1).unwrap(),
    });
    spec.name = ServiceName::parse("web").unwrap();
    spec.ports = vec![PortPublication::Ingress {
        hostname: IngressHostname::explicit("api.example.com").unwrap(),
        load_balancer_port: NonZeroU16::new(80).unwrap(),
        container_port: NonZeroU16::new(80).unwrap(),
        http_protocol: HttpProtocol::Http,
    }];
    spec
}

fn other_project_container(spec: &RequestedServiceSpec, created_at: i64) -> ContainerObservation {
    let mut observation = container('d', '1', spec, &service_id('b'));
    observation.project_name = ProjectName::parse("blog").unwrap();
    observation.created_at_unix_nanos = created_at;
    observation
}

fn snapshot_with(containers: Vec<ContainerObservation>) -> DeploySnapshot {
    DeploySnapshot {
        machines: vec![machine('1', "first")],
        containers,
        ..Default::default()
    }
}
