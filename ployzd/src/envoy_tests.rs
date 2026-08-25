//! Envoy renderer and apply-lifecycle contract tests.

use super::{
    Error as RendererError,
    apply::{ApplyIo, ApplyOutcome, Error as ApplyError, ValidationOutcome, apply},
    render, write_initial_config,
};
use crate::ingress::tests::renderer_projection;
use ployz_core::{ContainerAddress, IngressProxyFragment, MachineId, QualifiedService};
use std::{fs, path::Path, sync::Mutex};

const SELECTED_IMAGE: &str = "example.invalid/envoy@sha256:exact";

#[test]
fn caddy_fragment_is_a_typed_backend_mismatch() {
    let mut projection = renderer_projection();
    projection.service_fragments.insert(
        QualifiedService::parse("app/api").unwrap(),
        IngressProxyFragment::parse_caddy("respond ok").unwrap(),
    );

    assert!(matches!(
        render(&projection),
        Err(RendererError::BackendMismatch)
    ));
}

#[test]
fn shared_projection_matches_the_frozen_envoy_contract() {
    let rendered = render(&renderer_projection()).unwrap();

    assert_eq!(
        rendered.lds(),
        include_str!("envoy_tests/renderer_projection.lds.yaml")
    );
    assert_eq!(
        rendered.rds(),
        include_str!("envoy_tests/renderer_projection.rds.yaml")
    );
    assert_eq!(
        rendered.cds(),
        include_str!("envoy_tests/renderer_projection.cds.yaml")
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
        address: ContainerAddress("10.210.3.2".parse().unwrap()),
        port: std::num::NonZeroU16::new(8080).unwrap(),
    }]);

    let second = render(&projection).unwrap();

    assert!(second.cds().contains("address: 10.210.3.2"));
    assert!(!second.cds().contains("address: 10.210.1.2"));
    assert!(!second.cds().contains("address: 10.210.2.2"));
    assert_ne!(second.digest(), first.digest());
}

#[test]
fn http_routes_carry_upstream_timeouts_and_skip_https_only_sites() {
    let rendered = render(&renderer_projection()).unwrap();

    assert!(rendered.rds().contains("cluster: ployz-http-example.com"));
    assert!(rendered.rds().contains("timeout: 60s"));
    assert!(rendered.rds().contains("idle_timeout: 75s"));
    assert!(rendered.cds().contains("connect_timeout: 5s"));
    assert!(!rendered.rds().contains("secure.example.com"));
    assert!(!rendered.cds().contains("secure.example.com"));
    assert!(rendered.cds().contains("address: 127.0.0.1"));
    assert!(rendered.cds().contains("port_value: 1"));
    assert!(!rendered.lds().contains("admin:"));
    assert!(!rendered.rds().contains("acme-challenge"));
    assert!(
        rendered
            .lds()
            .contains(&format!("version_info: \"{}\"", rendered.digest()))
    );
    assert_eq!(
        rendered.rds().matches("timeout: 60s").count(),
        rendered.rds().matches("cluster: ployz-http-").count()
    );
}

#[test]
fn write_initial_config_is_idempotent_and_installs_file_watched_xds() {
    let root = test_root("bootstrap");
    let config_file = root.join("ingress/envoy/bootstrap.yaml");
    let machine = renderer_projection().machine;
    write_initial_config(&machine, &config_file).unwrap();
    let bootstrap = fs::read_to_string(&config_file).unwrap();
    assert_eq!(bootstrap, super::BOOTSTRAP);
    assert!(!bootstrap.contains("admin:"));
    assert!(bootstrap.contains("path_config_source:"));
    let rds = fs::read_to_string(config_file.parent().unwrap().join("rds.yaml")).unwrap();
    assert!(rds.contains("/.ployz-verify"));
    assert!(rds.contains(&machine.id.to_string()));
    let cds = fs::read_to_string(config_file.parent().unwrap().join("cds.yaml")).unwrap();
    assert!(cds.contains("resources: []"));
    assert!(cds.contains("version_info:"));
    fs::write(&config_file, "authoritative\n").unwrap();
    write_initial_config(&machine, &config_file).unwrap();
    assert_eq!(fs::read_to_string(&config_file).unwrap(), "authoritative\n");
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn rejected_candidate_leaves_live_configuration_untouched() {
    let root = test_root("rejected");
    let live = root.join("bootstrap.yaml");
    fs::create_dir_all(&root).unwrap();
    fs::write(&live, "live-bootstrap\n").unwrap();
    fs::write(root.join("lds.yaml"), "live-lds\n").unwrap();
    fs::write(root.join("rds.yaml"), "live-rds\n").unwrap();
    fs::write(root.join("cds.yaml"), "live-cds\n").unwrap();
    let io = FakeIo::rejecting("schema rejected");

    let error = apply(&renderer_projection(), &live, SELECTED_IMAGE, &io)
        .await
        .unwrap_err();

    let expected_digest = render(&renderer_projection()).unwrap().digest().to_owned();
    assert!(matches!(
        error,
        ApplyError::ValidationRejected { ref digest, .. } if digest == &expected_digest
    ));
    assert_eq!(fs::read_to_string(&live).unwrap(), "live-bootstrap\n");
    assert_eq!(
        fs::read_to_string(root.join("lds.yaml")).unwrap(),
        "live-lds\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("rds.yaml")).unwrap(),
        "live-rds\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("cds.yaml")).unwrap(),
        "live-cds\n"
    );
    assert!(!root.join(".apply-candidate").exists());
    let validations = io.validations.lock().unwrap();
    let (image, candidate) = validations.first().unwrap();
    assert_eq!(image, SELECTED_IMAGE);
    assert!(candidate.contains(&format!("projection_digest: {expected_digest}")));
    assert!(candidate.contains("timeout: 60s"));

    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn accepted_candidate_is_activated_without_admin_mutation() {
    let root = test_root("activated");
    let live = root.join("bootstrap.yaml");
    let projection = renderer_projection();
    let rendered = render(&projection).unwrap();
    let digest = rendered.digest().to_owned();
    let io = FakeIo::accepting();

    let outcome = apply(&projection, &live, SELECTED_IMAGE, &io)
        .await
        .unwrap();

    assert_eq!(outcome, ApplyOutcome::Activated { digest });
    assert_eq!(
        fs::read_to_string(root.join("lds.yaml")).unwrap(),
        rendered.lds()
    );
    assert_eq!(
        fs::read_to_string(root.join("rds.yaml")).unwrap(),
        rendered.rds()
    );
    assert_eq!(
        fs::read_to_string(root.join("cds.yaml")).unwrap(),
        rendered.cds()
    );
    assert!(!root.join(".apply-candidate").exists());

    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn docker_validation_errors_keep_their_typed_phase() {
    let projection = renderer_projection();
    let validation_root = test_root("validation-error");
    let validation_live = validation_root.join("bootstrap.yaml");
    let mut validation_io = FakeIo::accepting();
    validation_io.validation_error = true;

    let validation_error = apply(
        &projection,
        &validation_live,
        SELECTED_IMAGE,
        &validation_io,
    )
    .await
    .unwrap_err();
    assert!(matches!(validation_error, ApplyError::Validation(_)));
    assert!(!validation_root.join("lds.yaml").exists());

    fs::remove_dir_all(validation_root).unwrap();
}

struct FakeIo {
    validation: ValidationOutcome,
    validations: Mutex<Vec<(String, String)>>,
    validation_error: bool,
}

impl FakeIo {
    fn accepting() -> Self {
        Self {
            validation: ValidationOutcome::Accepted,
            validations: Mutex::default(),
            validation_error: false,
        }
    }

    fn rejecting(reason: &str) -> Self {
        Self {
            validation: ValidationOutcome::Rejected(reason.to_owned()),
            validations: Mutex::default(),
            validation_error: false,
        }
    }
}

impl ApplyIo for FakeIo {
    async fn validate_candidate(
        &self,
        image: &str,
        candidate: &Path,
    ) -> Result<ValidationOutcome, ApplyError> {
        self.validations.lock().unwrap().push((
            image.to_owned(),
            fs::read_to_string(candidate.join("rds.yaml")).unwrap(),
        ));
        if self.validation_error {
            return Err(ApplyError::Validation(
                crate::docker::Error::InvalidContainerConfig("test validation failure".into()),
            ));
        }
        Ok(self.validation.clone())
    }
}

fn test_root(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("ployz-envoy-apply-{label}-{}", MachineId::random()))
}
