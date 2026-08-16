use ployz_core::{HttpProtocol, IngressHost, IngressHostname, PortPublication};
use serde_json::json;

#[test]
fn ingress_host_rejects_empty_and_invalid_names() {
    assert_eq!(
        IngressHost::parse("").unwrap_err().to_string(),
        "invalid Ingress Hostname \"\": a 1-253 character lowercase DNS hostname"
    );
    assert!(IngressHost::parse("Example.com").is_err());
    assert!(IngressHost::parse("bad_host.example").is_err());
    assert!(IngressHost::parse(".example.com").is_err());
    assert_eq!(
        IngressHost::parse("app.example.com").unwrap().as_str(),
        "app.example.com"
    );
}

#[test]
fn explicit_ingress_hostname_rejects_empty_and_invalid_names() {
    assert_eq!(
        IngressHostname::explicit("").unwrap_err().to_string(),
        "invalid Ingress Hostname \"\": a 1-253 character lowercase DNS hostname"
    );
    assert!(IngressHostname::explicit("Example.com").is_err());
}

#[test]
fn ingress_hostname_intent_is_explicit_or_assigned() {
    assert_eq!(
        serde_json::to_value(IngressHostname::AssignFromClusterDomain).unwrap(),
        json!({ "kind": "assign_from_cluster_domain" })
    );
    assert_eq!(
        serde_json::to_value(IngressHostname::explicit("app.example.com").unwrap()).unwrap(),
        json!({ "kind": "explicit", "hostname": "app.example.com" })
    );
    assert!(serde_json::from_value::<PortPublication>(json!({
        "mode": "ingress",
        "hostname": "",
        "load_balancer_port": 80,
        "container_port": 8080,
        "http_protocol": "http"
    }))
    .is_err());
    assert!(serde_json::from_value::<PortPublication>(json!({
        "mode": "ingress_transport",
        "container_port": 53,
        "transport_protocol": "udp"
    }))
    .is_err());
    let assigned: PortPublication = serde_json::from_value(json!({
        "mode": "ingress",
        "hostname": { "kind": "assign_from_cluster_domain" },
        "load_balancer_port": 80,
        "container_port": 8080,
        "http_protocol": "http"
    }))
    .unwrap();
    assert!(matches!(
        assigned,
        PortPublication::Ingress {
            hostname: IngressHostname::AssignFromClusterDomain,
            http_protocol: HttpProtocol::Http,
            ..
        }
    ));
}
