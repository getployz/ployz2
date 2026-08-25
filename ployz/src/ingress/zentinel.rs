//! Concrete Zentinel deployment wiring for the Ingress Proxy.

use std::collections::BTreeMap;

use ployz_core::{
    ContainerPath, ContainerResources, MachinePath, MachineTarget, Placement, PullPolicy,
    QualifiedService, RequestedServiceSpec, RestartPolicy, ServiceContainerSpec, ServiceMode,
    ServiceMount, ServiceVolume, ServiceVolumeGraph, ServiceVolumeReference, UpdateConfig,
    UpdateOrder, VolumeSource, ZENTINEL_INGRESS_CAPABILITY, ZENTINEL_INGRESS_COMMAND,
};

use super::DATA_PATH;

/// Qualified Zentinel release selected for new Clusters.
pub const ZENTINEL_IMAGE: &str = "ghcr.io/zentinelproxy/zentinel@sha256:ff012547034d13a7d8e6570679c897e4bba6bc702ec5bdd7bf70a7a04b4d6604";

#[must_use]
pub(super) fn service_spec(image: String, machines: Vec<MachineTarget>) -> RequestedServiceSpec {
    let volume = ServiceVolumeReference::parse("ingress-data").expect("static volume is valid");
    let volume_graph = ServiceVolumeGraph::parse(
        vec![ServiceVolume {
            reference: volume.clone(),
            source: VolumeSource::Bind {
                machine_path: MachinePath::parse(format!("{DATA_PATH}/zentinel"))
                    .expect("static data path is valid"),
                create_machine_path: true,
                propagation: None,
                recursive: None,
            },
        }],
        vec![ServiceMount {
            volume,
            target: ContainerPath::parse("/config").expect("static mount path is valid"),
            read_only: false,
        }],
    )
    .expect("static Zentinel Volume graph is valid");
    RequestedServiceSpec {
        name: QualifiedService::system_ingress().name,
        mode: ServiceMode::Global,
        container: ServiceContainerSpec {
            image,
            command: ZENTINEL_INGRESS_COMMAND.map(str::to_owned).into(),
            entrypoint: Vec::new(),
            environment: BTreeMap::new(),
            labels: Default::default(),
            hostname: None,
            extra_hosts: Vec::new(),
            cap_add: vec![ZENTINEL_INGRESS_CAPABILITY.into()],
            cap_drop: vec!["ALL".into()],
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
        ports: Vec::new(),
        volume_graph,
        config_graph: Default::default(),
        pre_deploy: None,
        ingress_proxy_fragment: None,
        update: UpdateConfig {
            order: Some(UpdateOrder::StopFirst),
            ..Default::default()
        },
    }
}
