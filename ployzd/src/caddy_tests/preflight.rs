//! Candidate Caddy configuration preflight tests.

use super::{FakeAdmin, custom_observation, ingress};
use crate::caddy::preflight_candidate;
use ployz_core::{
    AdvertisedEndpoint, CaddyServiceConfig, HttpProtocol, Machine, MachineId, MachineName,
    ManagementAddress, QualifiedService, WireGuardPublicKey,
};
use std::collections::BTreeMap;

#[tokio::test]
async fn failed_preflight_adapts_fragments_without_loading() {
    let machine = machine();
    let services = [
        (
            QualifiedService::parse("app/api").unwrap(),
            present(&machine, 1, "api", "api.example { respond ok }"),
        ),
        (
            QualifiedService::parse("app/web").unwrap(),
            present(&machine, 2, "web", "# invalid\nweb.example { respond bad }"),
        ),
    ]
    .into();
    let admin = FakeAdmin::default();

    let error = preflight_candidate(&machine, &[], &BTreeMap::new(), &services, Some(&admin))
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Service 'app/web': Caddy adaptation failed: invalid config detected"
    );
    assert!(admin.loaded.lock().unwrap().is_empty());
    let adapted = admin.adapted.lock().unwrap();
    assert_eq!(adapted.len(), 2);
    assert!(adapted.last().unwrap().contains("api.example"));
    assert!(adapted.last().unwrap().contains("web.example"));
}

#[tokio::test]
async fn excludes_removed_services_from_upstream_resolution() {
    let machine = machine();
    let gateway = "gateway.example { reverse_proxy {{upstreams \"api\"}} }";
    let observations = vec![
        custom_observation(
            1,
            1,
            &machine.id,
            "api",
            "api.example { respond ok }",
            [10, 210, 1, 2],
        ),
        custom_observation(2, 1, &machine.id, "gateway", gateway, [10, 210, 1, 3]),
    ];
    let services = [
        (
            QualifiedService::parse("app/api").unwrap(),
            CaddyServiceConfig::Removed,
        ),
        (
            QualifiedService::parse("app/gateway").unwrap(),
            present(&machine, 3, "gateway", gateway),
        ),
    ]
    .into();

    let error = preflight_candidate(
        &machine,
        &observations,
        &BTreeMap::new(),
        &services,
        Some(&FakeAdmin::default()),
    )
    .await
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Service 'app/gateway': Caddy rendering failed: Service 'api' was not found"
    );
}

#[tokio::test]
async fn planned_ingress_routes_are_included_before_custom_adaptation() {
    let machine = machine();
    let mut spec = custom_observation(
        1,
        1,
        &machine.id,
        "web",
        "http://web.example { respond custom }",
        [10, 210, 1, 2],
    )
    .resolved_spec;
    spec.ports = vec![ingress("web.example", 8080, HttpProtocol::Http)];
    let services = [(
        QualifiedService::parse("app/web").unwrap(),
        CaddyServiceConfig::Present(Box::new(spec)),
    )]
    .into();
    let admin = FakeAdmin {
        reject_duplicate_site: Some("http://web.example".into()),
        ..Default::default()
    };

    let error = preflight_candidate(&machine, &[], &BTreeMap::new(), &services, Some(&admin))
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Service 'app/web': Caddy adaptation failed: duplicate site detected"
    );
    assert!(
        admin
            .adapted
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("reverse_proxy 127.0.0.1:8080")
    );
}

fn present(machine: &Machine, suffix: u8, service: &str, config: &str) -> CaddyServiceConfig {
    CaddyServiceConfig::Present(Box::new(
        custom_observation(
            suffix,
            1,
            &machine.id,
            service,
            config,
            [10, 210, 1, suffix],
        )
        .resolved_spec,
    ))
}

fn machine() -> Machine {
    Machine {
        id: MachineId::parse("a".repeat(32)).unwrap(),
        name: MachineName::parse("node-a").unwrap(),
        subnet: "10.210.1.0/24".parse().unwrap(),
        management_address: ManagementAddress("fdcc::1".parse().unwrap()),
        public_key: WireGuardPublicKey([1; 32]),
        public_ip: None,
        advertised_endpoints: vec![AdvertisedEndpoint("192.0.2.1:51000".parse().unwrap())],
        runtime: Default::default(),
    }
}
