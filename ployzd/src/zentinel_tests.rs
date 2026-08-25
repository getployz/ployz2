//! Zentinel renderer contract tests.

use super::{
    Error, ZENTINEL_BOOTSTRAP_CERT_FILE, ZENTINEL_BOOTSTRAP_KEY_FILE, ZENTINEL_CHALLENGES_DIR,
    ZENTINEL_GID, certificate_key_pair_matches, render, write_support_files,
};
use crate::{
    corrosion::CertificateChallenge,
    ingress::{certificate_file_stem, tests::renderer_projection},
};
use ployz_core::{IngressProxyFragment, MachineId, QualifiedService};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
};

#[test]
fn caddy_fragment_is_a_typed_backend_mismatch() {
    let mut projection = renderer_projection();
    projection.service_fragments.insert(
        QualifiedService::parse("app/api").unwrap(),
        IngressProxyFragment::parse_caddy("respond ok").unwrap(),
    );

    assert!(matches!(render(&projection), Err(Error::BackendMismatch)));
}

#[test]
fn shared_projection_matches_the_frozen_zentinel_contract() {
    let rendered = render(&renderer_projection()).unwrap();

    assert_eq!(
        rendered.kdl(),
        include_str!("zentinel_tests/renderer_projection.kdl")
    );
    assert_eq!(
        rendered.digest(),
        "7fea0e6032bb92f9cc4f67fd0838b5563fc40835d520ee514720bab4b9f7a052"
    );
}

#[test]
fn changed_projection_fully_regenerates_static_targets() {
    let mut projection = renderer_projection();
    let first = render(&projection).unwrap();
    let site = projection
        .sites
        .iter_mut()
        .find(|site| site.hostname.as_str() == "example.com")
        .unwrap();
    site.publication.as_mut().unwrap().http = Some(vec![crate::ingress::IngressEndpoint {
        address: ployz_core::ContainerAddress("10.210.3.2".parse().unwrap()),
        port: std::num::NonZeroU16::new(8080).unwrap(),
    }]);

    let second = render(&projection).unwrap();

    assert!(second.kdl().contains("target \"10.210.3.2:8080\""));
    assert!(!second.kdl().contains("target \"10.210.1.2:8080\""));
    assert!(!second.kdl().contains("target \"10.210.2.2:8080\""));
    assert_ne!(second.digest(), first.digest());
}

#[test]
fn support_files_are_stable_valid_and_readable_by_the_container() {
    let root = std::env::temp_dir().join(format!(
        "ployz-zentinel-support-test-{}",
        MachineId::random()
    ));
    let config_file = root.join("zentinel.kdl");
    let projection = renderer_projection();

    write_support_files(&projection, &config_file).unwrap();

    let certificates = root.join("certs");
    let bootstrap_certificate = certificates.join(ZENTINEL_BOOTSTRAP_CERT_FILE);
    let bootstrap_key = certificates.join(ZENTINEL_BOOTSTRAP_KEY_FILE);
    let certificate = fs::read_to_string(&bootstrap_certificate).unwrap();
    let key = fs::read_to_string(&bootstrap_key).unwrap();
    assert!(certificate_key_pair_matches(&certificate, &key));
    assert_eq!(mode(&certificates), 0o750);
    assert_eq!(mode(&bootstrap_certificate), 0o644);
    assert_eq!(mode(&bootstrap_key), 0o640);

    let secure_site = projection
        .sites
        .iter()
        .find(|site| site.hostname.as_str() == "secure.example.com")
        .unwrap();
    let material = secure_site.material().unwrap();
    let stem = certificate_file_stem(&secure_site.hostname, material);
    assert_eq!(mode(&certificates.join(format!("{stem}.crt"))), 0o644);
    assert_eq!(mode(&certificates.join(format!("{stem}.key"))), 0o640);

    let challenge = root
        .join(ZENTINEL_CHALLENGES_DIR)
        .join("example.com/.well-known/acme-challenge/token");
    assert_eq!(fs::read_to_string(&challenge).unwrap(), "token.thumbprint");
    assert_eq!(mode(&challenge), 0o644);
    assert_eq!(mode(&root.join(ZENTINEL_CHALLENGES_DIR)), 0o750);
    assert_eq!(mode(&root.join("challenges/example.com")), 0o750);
    assert_eq!(
        mode(&root.join("challenges/example.com/.well-known")),
        0o750
    );
    assert_eq!(
        mode(&root.join("challenges/example.com/.well-known/acme-challenge")),
        0o750
    );

    if fs::metadata("/proc/self").unwrap().uid() == 0 {
        assert_eq!(fs::metadata(&bootstrap_key).unwrap().gid(), ZENTINEL_GID);
        assert_eq!(fs::metadata(&challenge).unwrap().gid(), ZENTINEL_GID);
    }

    write_support_files(&projection, &config_file).unwrap();
    assert_eq!(
        fs::read_to_string(&bootstrap_certificate).unwrap(),
        certificate
    );

    fs::remove_file(&bootstrap_key).unwrap();
    write_support_files(&projection, &config_file).unwrap();
    assert!(certificate_key_pair_matches(
        &fs::read_to_string(&bootstrap_certificate).unwrap(),
        &fs::read_to_string(&bootstrap_key).unwrap()
    ));

    fs::write(&bootstrap_key, "not a private key").unwrap();
    let error = write_support_files(&projection, &config_file).unwrap_err();
    assert!(matches!(error, Error::InvalidBootstrapPair));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn support_files_reject_a_challenge_path() {
    let root = std::env::temp_dir().join(format!(
        "ployz-zentinel-challenge-path-test-{}",
        MachineId::random()
    ));
    let mut projection = renderer_projection();
    projection
        .sites
        .iter_mut()
        .find(|site| site.hostname.as_str() == "example.com")
        .unwrap()
        .certificate
        .as_mut()
        .unwrap()
        .challenge = CertificateChallenge::new("../token", "response");

    let error = write_support_files(&projection, &root.join("zentinel.kdl")).unwrap_err();

    assert!(matches!(error, Error::InvalidChallengeToken));
    assert!(!root.exists());
}

fn mode(path: &std::path::Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}
