//! Projection fixture shared by concrete renderer tests.

use super::{
    IngressEndpoint, IngressProjection, IngressPublication, IngressSite, ProjectedCertificate,
};
use crate::corrosion::{CertificateChallenge, CertificateMaterial};
use ployz_core::{
    ContainerAddress, IngressHost, IngressProxyBackend, Machine, MachineId, MachineName,
    ManagementAddress, QualifiedService, ResolvedServiceSpec, WireGuardPublicKey,
};
use serde_json::json;
use std::{collections::BTreeMap, num::NonZeroU16};

/// Projection that exercises HTTP, HTTPS, challenge, ordered, and empty routes.
pub(crate) fn renderer_projection() -> IngressProjection {
    let machine_id = MachineId::parse("a".repeat(32)).unwrap();
    let owner = QualifiedService::parse("app/api").unwrap();
    let endpoint = |address: &str, port| IngressEndpoint {
        address: ContainerAddress(address.parse().unwrap()),
        port: NonZeroU16::new(port).unwrap(),
    };
    let publication = |http, https| {
        Some(IngressPublication {
            owner: owner.clone(),
            http,
            https,
        })
    };
    IngressProjection {
        machine: Machine {
            id: machine_id,
            name: MachineName::parse("node-a").unwrap(),
            subnet: "10.210.1.0/24".parse().unwrap(),
            management_address: ManagementAddress("fdcc::1".parse().unwrap()),
            public_key: WireGuardPublicKey([1; 32]),
            public_ip: None,
            advertised_endpoints: Vec::new(),
            runtime: Default::default(),
        },
        sites: vec![
            IngressSite {
                hostname: IngressHost::parse("empty.example.com").unwrap(),
                publication: publication(Some(Vec::new()), None),
                certificate: None,
            },
            IngressSite {
                hostname: IngressHost::parse("example.com").unwrap(),
                publication: publication(
                    Some(vec![
                        endpoint("10.210.1.2", 8080),
                        endpoint("10.210.2.2", 8080),
                    ]),
                    None,
                ),
                certificate: Some(ProjectedCertificate {
                    challenge: CertificateChallenge::new("token", "token.thumbprint"),
                    material: None,
                    last_error: None,
                }),
            },
            IngressSite {
                hostname: IngressHost::parse("secure.example.com").unwrap(),
                publication: publication(None, Some(vec![endpoint("10.210.1.3", 8443)])),
                certificate: Some(ProjectedCertificate {
                    challenge: None,
                    material: CertificateMaterial::new("CERT", "KEY"),
                    last_error: None,
                }),
            },
        ],
        upstreams: BTreeMap::from([(
            owner,
            vec![
                ContainerAddress("10.210.1.2".parse().unwrap()),
                ContainerAddress("10.210.2.2".parse().unwrap()),
            ],
        )]),
        global_fragment: None,
        service_fragments: BTreeMap::new(),
    }
}

#[test]
fn reserved_ingress_service_backend_is_derived_from_concrete_wiring() {
    let caddy = resolved_spec(
        json!({
            "command": ["caddy", "run", "-c", "/config/caddy/Caddyfile"],
            "environment": {"CADDY_ADMIN": "unix//run/ingress/caddy/admin.sock"}
        }),
        json!([
            {"mode": "host", "bind": {"kind": "all"}, "published_port": 80, "container_port": 80, "transport_protocol": "tcp"},
            {"mode": "host", "bind": {"kind": "all"}, "published_port": 443, "container_port": 443, "transport_protocol": "tcp"},
            {"mode": "host", "bind": {"kind": "all"}, "published_port": 443, "container_port": 443, "transport_protocol": "udp"}
        ]),
    );
    let zentinel = resolved_spec(
        json!({
            "command": ["-c", "/config/zentinel.kdl"],
            "cap_add": ["NET_BIND_SERVICE"],
            "cap_drop": ["ALL"]
        }),
        json!([]),
    );

    assert_eq!(
        ployz_core::ingress_proxy_backend(&caddy).unwrap(),
        IngressProxyBackend::Caddy
    );
    assert_eq!(
        ployz_core::ingress_proxy_backend(&zentinel).unwrap(),
        IngressProxyBackend::Zentinel
    );
}

#[test]
fn reserved_ingress_service_backend_refuses_unknown_or_mixed_wiring() {
    let unknown = resolved_spec(json!({"command": ["proxy", "serve"]}), json!([]));
    let mixed = resolved_spec(
        json!({
            "command": ["-c", "/config/zentinel.kdl"],
            "cap_add": ["NET_BIND_SERVICE"],
            "cap_drop": ["ALL"]
        }),
        json!([]),
    );
    let mut mixed = mixed;
    mixed.ingress_proxy_fragment =
        Some(ployz_core::IngressProxyFragment::parse_caddy("respond ok").unwrap());

    assert!(ployz_core::ingress_proxy_backend(&unknown).is_err());
    assert!(ployz_core::ingress_proxy_backend(&mixed).is_err());
}

fn resolved_spec(container: serde_json::Value, ports: serde_json::Value) -> ResolvedServiceSpec {
    let mut base = json!({
        "service_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "name": "ingress",
        "mode": {"mode": "global"},
        "container": {
            "image": "example.test/ingress",
            "pull_policy": "missing"
        },
        "ports": ports
    });
    base.get_mut("container")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .extend(container.as_object().unwrap().clone());
    serde_json::from_value(base).unwrap()
}
