//! Concrete Caddy deployment wiring for the Ingress Proxy.

use std::{collections::BTreeMap, num::NonZeroU16};

use oci_client::{
    Client, ParseError, Reference, errors::OciDistributionError, secrets::RegistryAuth,
};
use ployz_core::{
    ContainerPath, ContainerResources, HostBind, IngressProxyFragment, MachinePath, MachineTarget,
    Placement, PortPublication, PullPolicy, QualifiedService, RequestedServiceSpec, RestartPolicy,
    ServiceContainer, ServiceContainerSpec, ServiceMode, ServiceMount, ServiceVolume,
    ServiceVolumeGraph, ServiceVolumeReference, TransportProtocol, UpdateConfig, VolumeSource,
};
use semver::Version;
use thiserror::Error;

use super::{DATA_PATH, RUNTIME_PATH, is_system_ingress};

/// Failure while discovering the current Caddy image for ingress deployment.
#[derive(Debug, Error)]
pub enum IngressImageError {
    /// The configured image reference could not be parsed.
    #[error("parse Caddy image reference: {0}")]
    Reference(#[from] ParseError),
    /// Docker Hub tags could not be listed.
    #[error("list Docker Hub Caddy tags: {0}")]
    ListTags(#[from] OciDistributionError),
}

/// Discover the latest stable Caddy 2 image used by the current ingress backend.
///
/// # Errors
///
/// Returns [`IngressImageError`] when the image reference is invalid or Docker
/// Hub cannot list its tags.
pub async fn latest_image() -> Result<String, IngressImageError> {
    let reference = "docker.io/library/caddy:latest".parse::<Reference>()?;
    let response = Client::default()
        .list_tags(&reference, &RegistryAuth::Anonymous, None, None)
        .await?;
    Ok(select_image(&response.tags))
}

#[must_use]
fn select_image(tags: &[String]) -> String {
    tags.iter()
        .filter_map(|tag| {
            Version::parse(tag).ok().filter(|version| {
                version.major == 2
                    && version.pre.is_empty()
                    && version.build.is_empty()
                    && version.to_string() == *tag
            })
        })
        .max()
        .map_or_else(
            || "caddy:latest".into(),
            |version| format!("caddy:{version}"),
        )
}

/// Recover the newest observed settings for the reserved Ingress Proxy Service.
#[must_use]
pub fn newest_existing_settings<'a>(
    containers: impl IntoIterator<Item = &'a ServiceContainer>,
) -> Option<(String, Vec<MachineTarget>, Option<String>)> {
    containers
        .into_iter()
        .filter(|container| is_system_ingress(container.as_observation()))
        .max_by_key(|container| {
            let observation = container.as_observation();
            (
                observation.created_at_unix_nanos,
                observation.container_id.as_str(),
            )
        })
        .map(|container| {
            let spec = &container.as_observation().resolved_spec;
            (
                spec.container.image.clone(),
                spec.placement.machines.clone(),
                spec.ingress_proxy_fragment
                    .as_ref()
                    .and_then(IngressProxyFragment::as_caddy)
                    .map(str::to_owned),
            )
        })
}

#[must_use]
/// Build the concrete Caddy Service Spec behind the neutral ingress identity.
pub fn service_spec(
    image: String,
    machines: Vec<MachineTarget>,
    caddy_config: Option<String>,
) -> RequestedServiceSpec {
    let volume = ServiceVolumeReference::parse("ingress-data").expect("static volume is valid");
    let runtime = ServiceVolumeReference::parse("ingress-runtime").expect("static volume is valid");
    let mount = |volume: &ServiceVolumeReference, target: &str| ServiceMount {
        volume: volume.clone(),
        target: ContainerPath::parse(target).expect("static mount path is valid"),
        read_only: false,
    };
    let host_port = |port, protocol| PortPublication::Host {
        bind: HostBind::All,
        published_port: NonZeroU16::new(port).expect("Caddy ports are non-zero"),
        container_port: NonZeroU16::new(port).expect("Caddy ports are non-zero"),
        transport_protocol: protocol,
    };
    let volume_graph = ServiceVolumeGraph::parse(
        vec![
            ServiceVolume {
                reference: volume.clone(),
                source: VolumeSource::Bind {
                    machine_path: MachinePath::parse(DATA_PATH).expect("static data path is valid"),
                    create_machine_path: true,
                    propagation: None,
                    recursive: None,
                },
            },
            ServiceVolume {
                reference: runtime.clone(),
                source: VolumeSource::Bind {
                    machine_path: MachinePath::parse(RUNTIME_PATH)
                        .expect("static runtime path is valid"),
                    create_machine_path: true,
                    propagation: None,
                    recursive: None,
                },
            },
        ],
        vec![
            mount(&volume, "/config"),
            mount(&volume, "/data"),
            mount(&runtime, "/run/ingress"),
        ],
    )
    .expect("static Caddy Volume graph is valid");
    RequestedServiceSpec {
        name: QualifiedService::system_ingress().name,
        mode: ServiceMode::Global,
        container: ServiceContainerSpec {
            image,
            command: vec![
                "caddy".into(),
                "run".into(),
                "-c".into(),
                "/config/caddy/Caddyfile".into(),
            ],
            entrypoint: Vec::new(),
            environment: BTreeMap::from([(
                "CADDY_ADMIN".into(),
                "unix//run/ingress/caddy/admin.sock".into(),
            )]),
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
            sysctls: BTreeMap::new(),
            restart: RestartPolicy::default(),
        },
        placement: Placement { machines },
        ports: vec![
            host_port(80, TransportProtocol::Tcp),
            host_port(443, TransportProtocol::Tcp),
            host_port(443, TransportProtocol::Udp),
        ],
        volume_graph,
        config_graph: Default::default(),
        pre_deploy: None,
        ingress_proxy_fragment: caddy_config
            .filter(|config| !config.trim().is_empty())
            .map(IngressProxyFragment::parse_caddy)
            .transpose()
            .expect("non-empty Caddy configuration is valid"),
        update: UpdateConfig::default(),
    }
}

#[cfg(test)]
mod tests {
    use ployz_core::{
        ContainerId, ContainerKind, ContainerObservation, MachineTarget, PortPublication,
        ServiceContainer, ServiceName, TransportProtocol, service_containers,
    };

    use super::*;

    #[test]
    fn selects_only_the_greatest_bare_two_x_y_tag() {
        assert_eq!(
            select_image(&[
                "2.9.1".into(),
                "2.10.0".into(),
                "2.11.0-rc.1".into(),
                "2.10".into(),
                "latest".into(),
                "3.0.0".into(),
            ]),
            "caddy:2.10.0"
        );
        assert_eq!(select_image(&["latest".into()]), "caddy:latest");
    }

    #[test]
    fn machine_add_reuses_the_newest_observed_ingress_settings() {
        let mut older: ContainerObservation = serde_json::from_value(serde_json::json!({
            "container_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "display_name": "ingress-old",
            "machine_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "project_name": "ployz-system",
            "service_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "service_name": "ingress",
            "kind": "service_container",
            "runtime": { "state": "created" },
            "resolved_spec": {
                "service_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "name": "ingress",
                "mode": { "mode": "global" },
                "container": { "image": "caddy:2.9.1", "pull_policy": "missing" }
            }
        }))
        .unwrap();
        older.created_at_unix_nanos = 1;
        let mut newer = older.clone();
        newer.container_id = ployz_core::ContainerId::parse("b".repeat(64)).unwrap();
        newer.created_at_unix_nanos = 2;
        newer.resolved_spec.container.image = "caddy:2.10.2".into();
        newer.resolved_spec.placement.machines = vec![MachineTarget::parse("edge").unwrap()];
        newer.resolved_spec.ingress_proxy_fragment =
            Some(IngressProxyFragment::parse_caddy("{ admin off }").expect("fixture is non-empty"));
        let mut hook = newer.clone();
        hook.kind = ContainerKind::PreDeployHook;
        hook.container_id = ContainerId::parse("c".repeat(64)).unwrap();
        hook.created_at_unix_nanos = 3;
        hook.resolved_spec.container.image = "caddy:hook".into();
        let mut user = newer.clone();
        user.project_name = ployz_core::ProjectName::parse("shop").unwrap();
        user.container_id = ContainerId::parse("d".repeat(64)).unwrap();
        user.created_at_unix_nanos = 4;
        user.resolved_spec.container.image = "caddy:user".into();

        assert_eq!(
            newest_existing_settings(&service_containers([user, newer, older, hook])),
            Some((
                "caddy:2.10.2".into(),
                vec![MachineTarget::parse("edge").unwrap()],
                Some("{ admin off }".into())
            ))
        );
        assert_eq!(newest_existing_settings(&[] as &[ServiceContainer]), None);
    }

    #[test]
    fn service_spec_uses_neutral_identity_roots_and_concrete_caddy_wiring() {
        let spec = service_spec("caddy:2.10.0".into(), Vec::new(), None);

        assert_eq!(spec.name, ServiceName::parse("ingress").unwrap());
        assert_eq!(spec.mode, ServiceMode::Global);
        assert_eq!(
            spec.container.command,
            ["caddy", "run", "-c", "/config/caddy/Caddyfile"]
        );
        assert_eq!(
            spec.container
                .environment
                .get("CADDY_ADMIN")
                .map(String::as_str),
            Some("unix//run/ingress/caddy/admin.sock")
        );
        assert_eq!(spec.ports.len(), 3);
        assert!(matches!(
            spec.ports.get(2),
            Some(PortPublication::Host {
                transport_protocol: TransportProtocol::Udp,
                ..
            })
        ));
        assert_eq!(spec.mounts().len(), 3);
        assert_eq!(
            spec.volume_graph
                .volumes()
                .iter()
                .filter_map(|volume| match &volume.source {
                    VolumeSource::Bind { machine_path, .. } => Some(machine_path.as_str()),
                    VolumeSource::Named { .. } | VolumeSource::Tmpfs { .. } => None,
                })
                .collect::<Vec<_>>(),
            [DATA_PATH, RUNTIME_PATH]
        );
        assert!(spec.config_graph.mounts().is_empty());
    }
}
