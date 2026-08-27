use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ployz_core::{
    CertificateKeyType, CertificatePolicy, ContainerAddress, ContainerId, ContainerKind,
    ContainerObservation, ContainerRuntimeObservation, DEFAULT_RENEW_AT_LIFETIME_FRACTION,
    HealthObservation, HttpProtocol, IngressHost, IngressHostname, IssuanceClock, IssuanceFailure,
    IssuanceGate, Machine, MachineId, PortPublication, ProjectName, ResolvedServiceSpec, ServiceId,
    ServiceName, resolve_certificate_policy,
};
use serde_json::json;

use super::{
    CHALLENGE_WAIT, IssuanceAction, STEAL_AFTER, allocator_stalled_for, challenge_probe_addresses,
    contacts_authority, directory_from_env, ingress_challenge_ips, issuance_action,
    material_validity, order_certificate, poll_wait, renewal_window, steal_due, wait_for_http01,
    wanted_certificate_hosts,
};
use crate::corrosion::{AllocatorView, CertificateChallenge, CertificateMaterial, CertificateRow};

fn policy_for(directory: &str) -> CertificatePolicy {
    CertificatePolicy::built_in(Some(directory.to_owned()))
}

fn default_policy() -> CertificatePolicy {
    CertificatePolicy::built_in(None)
}

fn policy_probe(seconds: u64) -> CertificatePolicy {
    resolve_certificate_policy(
        Some(&format!(r#"{{"probe_timeout":{seconds}}}"#)),
        &CertificatePolicy::built_in(None),
    )
    .unwrap()
}

#[test]
fn directory_empty_disables_issuance() {
    assert_eq!(directory_from_env(Some("")), None);
    assert_eq!(
        directory_from_env(Some("http://ca.test/directory")).as_deref(),
        Some("http://ca.test/directory")
    );
    assert_eq!(
        directory_from_env(None).as_deref(),
        Some("https://acme-v02.api.letsencrypt.org/directory")
    );
}

#[test]
fn wanted_hosts_are_https_ingress_only() {
    let observations = [
        observation(
            1,
            "api",
            vec![
                ingress("app.example.com", HttpProtocol::Https),
                ingress("plain.example.com", HttpProtocol::Http),
                ingress("web.opaque.uncloud.example", HttpProtocol::Https),
            ],
        ),
        observation(2, "www", vec![ingress_assign(HttpProtocol::Https)]),
        observation(4, "edge", vec![ingress_chosen(HttpProtocol::Https)]),
        {
            let mut hook = observation(
                3,
                "api",
                vec![ingress("hook.example.com", HttpProtocol::Https)],
            );
            hook.kind = ContainerKind::PreDeployHook;
            hook
        },
    ];

    assert_eq!(
        wanted_certificate_hosts(observations.iter()),
        BTreeSet::from([host("app.example.com"), host("web.opaque.uncloud.example"),])
    );
    assert_eq!(
        wanted_certificate_hosts(
            [observation(
                1,
                "api",
                vec![ingress("plain.example.com", HttpProtocol::Http)]
            )]
            .iter()
        ),
        BTreeSet::new()
    );
    assert_eq!(
        wanted_certificate_hosts([observation(1, "api", Vec::new())].iter()),
        BTreeSet::new()
    );
}

#[test]
fn missing_material_orders_immediately() {
    assert_eq!(decide(None), IssuanceAction::Order);
    assert_eq!(
        issuance_action(None, UNIX_EPOCH, &policy_probe(120)),
        IssuanceAction::Order
    );
}

#[test]
fn an_attended_row_is_never_stalled() {
    let policy = default_policy();
    let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
    let wanting = STEAL_AFTER * 10;
    // A refusing-but-alive Allocator keeps the clock's next attempt ahead of now.
    let attended = CertificateRow::from_parts(None, None).with_backoff(
        "does not resolve",
        IssuanceClock::new(
            3,
            now + Duration::from_secs(60),
            IssuanceFailure::DoesNotResolve,
        ),
    );
    assert_eq!(
        allocator_stalled_for(Some(&attended), wanting, now, &policy),
        Duration::ZERO
    );
    // A dead Allocator leaves the clock frozen.
    let frozen = CertificateRow::from_parts(None, None).with_backoff(
        "authority failed",
        IssuanceClock::new(3, now - STEAL_AFTER * 2, IssuanceFailure::Authority),
    );
    assert_eq!(
        allocator_stalled_for(Some(&frozen), wanting, now, &policy),
        STEAL_AFTER * 2
    );
    // A renewal window the Allocator never attended stalls from the window.
    let day = Duration::from_secs(86_400);
    let expired = row_with_lifetime(UNIX_EPOCH, UNIX_EPOCH + day * 90);
    let window = renewal_window(
        UNIX_EPOCH,
        UNIX_EPOCH + day * 90,
        policy.renew_at_lifetime_fraction(),
    )
    .unwrap();
    let later = window + STEAL_AFTER * 3;
    assert_eq!(
        allocator_stalled_for(Some(&expired), Duration::ZERO, later, &policy),
        STEAL_AFTER * 3
    );
    // A failed renewal's clock attends a material row: stall follows the clock.
    let renewing = expired.with_backoff(
        "authority failed",
        IssuanceClock::new(
            1,
            later + Duration::from_secs(60),
            IssuanceFailure::Authority,
        ),
    );
    assert_eq!(
        allocator_stalled_for(Some(&renewing), Duration::ZERO, later, &policy),
        Duration::ZERO
    );
    // An untouched row falls back to this Machine's own observation age.
    assert_eq!(allocator_stalled_for(None, wanting, now, &policy), wanting);
}

#[test]
fn only_a_stalled_non_allocator_steals() {
    let other = AllocatorView::Other(machine_id("f"));
    let stalled = STEAL_AFTER + Duration::from_secs(1);
    assert!(steal_due(&other, IssuanceAction::Order, stalled));
    assert!(steal_due(
        &AllocatorView::Vacant,
        IssuanceAction::Renew,
        stalled
    ));
    assert!(!steal_due(&other, IssuanceAction::Order, STEAL_AFTER));
    assert!(!steal_due(&other, IssuanceAction::Nothing, stalled));
    assert!(!steal_due(
        &AllocatorView::Held,
        IssuanceAction::Order,
        stalled
    ));
    assert!(!steal_due(
        &AllocatorView::HeldNotQuiet,
        IssuanceAction::Order,
        stalled
    ));
}

#[test]
fn renew_does_not_contact_the_authority_when_dns_refuses() {
    let clock = IssuanceClock::new(1, UNIX_EPOCH, IssuanceFailure::ResolvesElsewhere);
    assert!(!contacts_authority(
        IssuanceAction::Renew,
        IssuanceGate::Refuse(clock)
    ));
    assert!(!contacts_authority(
        IssuanceAction::Order,
        IssuanceGate::Refuse(clock)
    ));
    assert!(!contacts_authority(
        IssuanceAction::Renew,
        IssuanceGate::Nothing
    ));
    assert!(contacts_authority(
        IssuanceAction::Renew,
        IssuanceGate::Order
    ));
    assert!(contacts_authority(
        IssuanceAction::Order,
        IssuanceGate::Order
    ));
    assert!(!contacts_authority(
        IssuanceAction::Nothing,
        IssuanceGate::Order
    ));
}

#[test]
fn unparseable_material_does_not_renew() {
    let material = CertificateMaterial::new("CERT", "KEY").unwrap();
    let row = CertificateRow::from_parts(Some(material), None);
    assert_eq!(decide(Some(&row)), IssuanceAction::Nothing);
}

#[test]
fn renewal_window_is_two_thirds_of_the_certificate_lifetime() {
    let day = Duration::from_secs(86_400);
    let start = UNIX_EPOCH;
    assert_eq!(
        renewal_window(start, start + day * 90, DEFAULT_RENEW_AT_LIFETIME_FRACTION).unwrap(),
        start + day * 60
    );
    assert_eq!(
        renewal_window(start, start + day * 6, DEFAULT_RENEW_AT_LIFETIME_FRACTION).unwrap(),
        start + day * 4
    );
}

#[test]
fn renewal_window_uses_the_policy_fraction() {
    let start = UNIX_EPOCH;
    let life = Duration::from_secs(100);
    assert_eq!(
        renewal_window(start, start + life, 0.5).unwrap(),
        start + Duration::from_secs(50)
    );
}

#[test]
fn same_certificate_renews_at_two_thirds_for_long_and_short_lifetimes() {
    let day = Duration::from_secs(86_400);
    let policy = default_policy();
    for lifetime in [day * 90, day * 6] {
        let not_before = UNIX_EPOCH;
        let not_after = not_before + lifetime;
        let row = row_with_lifetime(not_before, not_after);
        let window =
            renewal_window(not_before, not_after, policy.renew_at_lifetime_fraction()).unwrap();
        assert_eq!(
            issuance_action(Some(&row), window - Duration::from_secs(1), &policy),
            IssuanceAction::Nothing,
            "lifetime {lifetime:?}"
        );
        assert_eq!(
            issuance_action(Some(&row), window, &policy),
            IssuanceAction::Renew,
            "lifetime {lifetime:?}"
        );
        assert_eq!(
            issuance_action(Some(&row), not_after, &policy),
            IssuanceAction::Renew,
            "lifetime {lifetime:?}"
        );
    }
}

#[test]
fn poll_wait_follows_each_certificate_lifetime() {
    let day = Duration::from_secs(86_400);
    let policy = default_policy();
    let long = row_with_lifetime(UNIX_EPOCH, UNIX_EPOCH + day * 90);
    let short = row_with_lifetime(UNIX_EPOCH, UNIX_EPOCH + day * 6);
    let now = UNIX_EPOCH + Duration::from_secs(60);
    assert_eq!(
        poll_wait(Some(&long), now, &policy),
        Duration::from_secs(60)
    );
    assert_eq!(
        poll_wait(Some(&short), now, &policy),
        Duration::from_secs(60)
    );
    let near_short = super::renew_at(
        short.material().unwrap(),
        policy.renew_at_lifetime_fraction(),
    )
    .unwrap()
        - Duration::from_secs(10);
    assert_eq!(
        poll_wait(Some(&short), near_short, &policy),
        Duration::from_secs(10)
    );
    let past_short = near_short + Duration::from_secs(11);
    assert_eq!(poll_wait(Some(&short), past_short, &policy), CHALLENGE_WAIT);
}

#[test]
fn probe_addresses_are_the_ingress_intersection() {
    let ingress_ips = BTreeSet::from([ip("192.0.2.1"), ip("192.0.2.2")]);
    assert_eq!(
        challenge_probe_addresses(&[ip("192.0.2.2"), ip("198.51.100.10")], &ingress_ips),
        vec![socket("192.0.2.2")]
    );
    assert_eq!(
        challenge_probe_addresses(&[ip("198.51.100.10")], &ingress_ips),
        Vec::<SocketAddr>::new()
    );
    assert_eq!(
        challenge_probe_addresses(&[], &ingress_ips),
        Vec::<SocketAddr>::new()
    );
}

#[test]
fn ingress_challenge_ips_come_from_running_ingress_machines() {
    let local = machine_with_endpoint("a", "192.0.2.1");
    let remote = machine_with_endpoint("b", "192.0.2.2");
    let mut ingress = observation(1, "ingress", Vec::new());
    ingress.machine_id = local.id;
    ingress.service_name = ServiceName::parse("ingress").unwrap();
    ingress.project_name = ProjectName::system();
    let mut down = observation(2, "ingress", Vec::new());
    down.machine_id = remote.id;
    down.service_name = ServiceName::parse("ingress").unwrap();
    down.project_name = ProjectName::system();
    down.runtime = ContainerRuntimeObservation::Exited { code: 1 };
    let mut user = observation(3, "caddy", Vec::new());
    user.machine_id = remote.id;
    assert_eq!(
        ingress_challenge_ips(&[local, remote], &[ingress, down, user]),
        BTreeSet::from([ip("192.0.2.1")])
    );
}

#[tokio::test]
async fn challenge_must_be_answerable_on_every_probe_address() {
    let hostname = host("app.example.com");
    let challenge = CertificateChallenge::new("tok", "tok.thumb").unwrap();
    let answers = std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::from([(
        "tok".to_owned(),
        "tok.thumb".to_owned(),
    )])));
    let (first_stop, first_port) =
        ployz_testkit::fake_acme::serve_http01(std::sync::Arc::clone(&answers));
    let (second_stop, second_port) = ployz_testkit::fake_acme::serve_http01(answers);
    wait_for_http01(
        &hostname,
        &challenge,
        &[
            SocketAddr::from(([127, 0, 0, 1], first_port)),
            SocketAddr::from(([127, 0, 0, 1], second_port)),
        ],
        CHALLENGE_WAIT,
    )
    .await
    .unwrap();
    drop((first_stop, second_stop));
}

#[tokio::test]
async fn empty_probe_addresses_fail_without_waiting() {
    let hostname = host("app.example.com");
    let challenge = CertificateChallenge::new("tok", "tok.thumb").unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        wait_for_http01(&hostname, &challenge, &[], CHALLENGE_WAIT),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(matches!(error, super::Error::ChallengeNotServed));
}

#[tokio::test]
async fn custom_https_hostname_obtains_a_certificate_from_a_fake_ca() {
    let hostname = host("app.example.com");
    let answers = std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::new()));
    let (http01, validation_port) =
        ployz_testkit::fake_acme::serve_http01(std::sync::Arc::clone(&answers));
    let ca = ployz_testkit::fake_acme::FakeCa::bind("127.0.0.1:0")
        .await
        .unwrap();
    ca.set_validation("127.0.0.1", validation_port);
    let account_dir =
        std::env::temp_dir().join(format!("ployzd-acme-{}-{}", std::process::id(), hostname));
    let policy = resolve_certificate_policy(
        Some(&format!(r#"{{"directory_url":"{}"}}"#, ca.directory_url())),
        &CertificatePolicy::built_in(None),
    )
    .unwrap();
    let material = order_certificate(&hostname, &policy, &account_dir, |challenge| {
        let answers = std::sync::Arc::clone(&answers);
        async move {
            answers.lock().unwrap().insert(
                challenge.token().to_owned(),
                challenge.response().to_owned(),
            );
            Ok(())
        }
    })
    .await
    .unwrap();

    assert!(material.certificate().contains("BEGIN CERTIFICATE"));
    assert!(material.private_key().contains("BEGIN"));
    assert_eq!(ca.ordered(), vec!["app.example.com".to_owned()]);
    drop(http01);
    let _ = std::fs::remove_dir_all(account_dir);
}

#[tokio::test]
async fn policy_key_type_and_eab_issue_against_a_fake_ca() {
    let hostname = host("rsa.example.com");
    let answers = std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::new()));
    let (http01, validation_port) =
        ployz_testkit::fake_acme::serve_http01(std::sync::Arc::clone(&answers));
    let ca = ployz_testkit::fake_acme::FakeCa::bind("127.0.0.1:0")
        .await
        .unwrap();
    ca.set_validation("127.0.0.1", validation_port);
    let account_dir = std::env::temp_dir().join(format!(
        "ployzd-acme-policy-{}-{}",
        std::process::id(),
        hostname
    ));
    let policy = resolve_certificate_policy(
        Some(&format!(
            r#"{{
                    "directory_url":"{}",
                    "eab":{{"kid":"kid-1","hmac_key":"dGVzdA"}},
                    "key_type":"ecdsa-p384",
                    "probe_timeout":5
                }}"#,
            ca.directory_url()
        )),
        &CertificatePolicy::built_in(None),
    )
    .unwrap();
    assert_eq!(policy.key_type(), &CertificateKeyType::EcdsaP384);
    assert_eq!(policy.eab().unwrap().kid(), "kid-1");
    let material = order_certificate(&hostname, &policy, &account_dir, |challenge| {
        let answers = std::sync::Arc::clone(&answers);
        async move {
            answers.lock().unwrap().insert(
                challenge.token().to_owned(),
                challenge.response().to_owned(),
            );
            Ok(())
        }
    })
    .await
    .unwrap();

    assert!(material.certificate().contains("BEGIN CERTIFICATE"));
    assert!(material.private_key().contains("BEGIN"));
    assert_eq!(ca.ordered(), vec!["rsa.example.com".to_owned()]);
    drop(http01);
    let _ = std::fs::remove_dir_all(account_dir);
}

#[tokio::test]
async fn fake_ca_lifetime_is_the_certificate_lifetime() {
    for lifetime in [
        Duration::from_secs(90 * 86_400),
        Duration::from_secs(6 * 86_400),
    ] {
        let hostname = host("app.example.com");
        let answers = std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let (http01, validation_port) =
            ployz_testkit::fake_acme::serve_http01(std::sync::Arc::clone(&answers));
        let ca = ployz_testkit::fake_acme::FakeCa::bind("127.0.0.1:0")
            .await
            .unwrap();
        ca.set_validation("127.0.0.1", validation_port);
        ca.set_certificate_lifetime(lifetime);
        let account_dir = std::env::temp_dir().join(format!(
            "ployzd-acme-life-{}-{}-{}",
            std::process::id(),
            lifetime.as_secs(),
            hostname
        ));
        let policy = policy_for(&ca.directory_url());
        let material = order_certificate(&hostname, &policy, &account_dir, |challenge| {
            let answers = std::sync::Arc::clone(&answers);
            async move {
                answers.lock().unwrap().insert(
                    challenge.token().to_owned(),
                    challenge.response().to_owned(),
                );
                Ok(())
            }
        })
        .await
        .unwrap();
        let (not_before, not_after) = material_validity(material.certificate()).unwrap();
        let got = not_after.duration_since(not_before).unwrap();
        assert!(
            got >= lifetime - Duration::from_secs(2) && got <= lifetime + Duration::from_secs(2),
            "lifetime {lifetime:?} got {got:?}"
        );
        drop(http01);
        let _ = std::fs::remove_dir_all(account_dir);
    }
}

#[tokio::test]
async fn order_fails_when_the_directory_is_unreachable() {
    let hostname = host("app.example.com");
    let account_dir = std::env::temp_dir().join(format!(
        "ployzd-acme-unreachable-{}-{}",
        std::process::id(),
        hostname
    ));
    let error = order_certificate(
        &hostname,
        &policy_for("http://127.0.0.1:1/directory"),
        &account_dir,
        |_| async { Ok(()) },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, super::Error::Acme(_) | super::Error::Http(_)),
        "{error}"
    );
    let _ = std::fs::remove_dir_all(account_dir);
}

fn decide(row: Option<&CertificateRow>) -> IssuanceAction {
    issuance_action(row, UNIX_EPOCH, &default_policy())
}

fn machine_id(seed: &str) -> MachineId {
    MachineId::parse(seed.repeat(32)).unwrap()
}

fn row_with_lifetime(not_before: SystemTime, not_after: SystemTime) -> CertificateRow {
    let (certificate, private_key) =
        ployz_testkit::fake_acme::self_signed_material("app.example.com", not_before, not_after);
    let material = CertificateMaterial::new(certificate, private_key).unwrap();
    CertificateRow::from_parts(Some(material), None)
}

fn host(name: &str) -> IngressHost {
    IngressHost::parse(name).unwrap()
}

fn ip(value: &str) -> IpAddr {
    value.parse().unwrap()
}

fn socket(value: &str) -> SocketAddr {
    SocketAddr::new(ip(value), 80)
}

fn machine_with_endpoint(seed: &str, address: &str) -> Machine {
    serde_json::from_value(json!({
        "id": seed.repeat(32),
        "name": format!("machine-{seed}"),
        "subnet": "10.210.1.0/24",
        "management_address": "fdcc::1",
        "public_key": vec![3; 32],
        "advertised_endpoints": [format!("{address}:51000")],
    }))
    .unwrap()
}

fn ingress(hostname: &str, http_protocol: HttpProtocol) -> PortPublication {
    PortPublication::Ingress {
        hostname: IngressHostname::explicit(hostname).unwrap(),
        load_balancer_port: 443.try_into().unwrap(),
        container_port: 8080.try_into().unwrap(),
        http_protocol,
    }
}

fn ingress_assign(http_protocol: HttpProtocol) -> PortPublication {
    PortPublication::Ingress {
        hostname: IngressHostname::cluster_domain(),
        load_balancer_port: 443.try_into().unwrap(),
        container_port: 8080.try_into().unwrap(),
        http_protocol,
    }
}

fn ingress_chosen(http_protocol: HttpProtocol) -> PortPublication {
    PortPublication::Ingress {
        hostname: IngressHostname::cluster_domain_label("api").unwrap(),
        load_balancer_port: 443.try_into().unwrap(),
        container_port: 8080.try_into().unwrap(),
        http_protocol,
    }
}

fn observation(
    suffix: u8,
    service_name: &str,
    ports: Vec<PortPublication>,
) -> ContainerObservation {
    let service_id = ServiceId::parse(format!("{suffix:x}").repeat(32)).unwrap();
    let service_name = ServiceName::parse(service_name).unwrap();
    let resolved_spec: ResolvedServiceSpec = serde_json::from_value(json!({
        "service_id": service_id,
        "name": service_name,
        "mode": { "mode": "replicated", "replicas": 1 },
        "container": { "image": "example.test/image", "pull_policy": "missing" },
        "ports": ports,
    }))
    .unwrap();
    ContainerObservation {
        container_id: ContainerId::parse(format!("{suffix:x}").repeat(64)).unwrap(),
        display_name: format!("{service_name}-{suffix}"),
        created_at_unix_nanos: 0,
        machine_id: MachineId::parse("a".repeat(32)).unwrap(),
        project_name: ProjectName::parse("app").unwrap(),
        service_id,
        service_name,
        kind: ContainerKind::ServiceContainer,
        runtime: ContainerRuntimeObservation::Running {
            health: HealthObservation::Healthy,
        },
        effective_healthcheck: None,
        resolved_spec,
        address: Some(ContainerAddress([10, 210, 1, 2].into())),
        labels: BTreeMap::new(),
    }
}
