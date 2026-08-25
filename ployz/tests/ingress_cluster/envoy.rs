//! Envoy founding tracer against the privileged Layer 3 cluster.

use std::{path::Path, process, time::Duration};

use ployz_core::{
    ContainerAction, IngressProxyBackend, InspectRequest, ResolvedServiceSpec, ServiceId, op,
};
use ployz_testkit::{Cluster, ClusterPlan};

use super::{
    change_container, cli, create_and_start, request, wait_config, wait_log_count, wait_running,
    wait_service,
};

#[tokio::test]
#[ignore = "Layer 3: requires the privileged Ployz testkit image"]
async fn envoy_founding_deploys_a_healthy_pinned_bridge_proxy() {
    let plan = ClusterPlan::new(&format!("l3-envoy-{}", process::id()), 1).unwrap();
    let cluster = Cluster::create(plan).unwrap();
    cluster.wait_ready(Duration::from_secs(30)).await.unwrap();
    let machine = cluster
        .initialize_first_with_backend(IngressProxyBackend::Envoy)
        .await
        .unwrap();
    cluster.wait_ready(Duration::from_secs(60)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if cluster
                .machines(0)
                .await
                .is_ok_and(|machines| machines.iter().any(|entry| entry.machine.id == machine.id))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap();
    let direct = cluster.api_address(0).unwrap();
    let mut client =
        ployz::connect::connect(Path::new("/missing-ployz-test-config"), Some(&direct), None)
            .await
            .unwrap();

    let inspected = client
        .call::<op::Inspect>(InspectRequest::default(), None)
        .await
        .unwrap();
    assert_eq!(
        inspected.ingress_proxy_backend,
        Some(IngressProxyBackend::Envoy)
    );

    cli(&direct, &["ingress", "deploy"]);
    let ingress_id = wait_service(&mut client, "ingress", 1).await;
    let ingress = wait_running(&mut client, &ingress_id, 1).await.remove(0);
    assert_eq!(
        ingress.resolved_spec.container.image,
        ployz::ingress::ENVOY_IMAGE
    );
    assert_eq!(ingress.resolved_spec.ports.len(), 2);
    cluster
        .machine_shell(
            0,
            &format!(
                "test \"$(docker inspect --format '{{{{.HostConfig.NetworkMode}}}}' {})\" != host",
                ingress.container_id
            ),
        )
        .unwrap();
    cluster
        .machine_shell(
            0,
            &format!(
                "test \"$(docker inspect --format '{{{{(index (index .HostConfig.PortBindings \"8080/tcp\") 0).HostPort}}}}' {})\" = 80",
                ingress.container_id
            ),
        )
        .unwrap();
    cluster
        .machine_shell(
            0,
            &format!(
                "test \"$(docker inspect --format '{{{{(index (index .HostConfig.PortBindings \"8443/tcp\") 0).HostPort}}}}' {})\" = 443",
                ingress.container_id
            ),
        )
        .unwrap();
}

#[tokio::test]
#[ignore = "Layer 3: requires the privileged Ployz testkit image"]
async fn envoy_routes_http_through_file_watched_xds() {
    let plan = ClusterPlan::new(&format!("l3-envoy-http-{}", process::id()), 1).unwrap();
    let cluster = Cluster::create(plan).unwrap();
    cluster.wait_ready(Duration::from_secs(30)).await.unwrap();
    let machine = cluster
        .initialize_first_with_backend(IngressProxyBackend::Envoy)
        .await
        .unwrap();
    cluster.wait_ready(Duration::from_secs(60)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if cluster
                .machines(0)
                .await
                .is_ok_and(|machines| machines.iter().any(|entry| entry.machine.id == machine.id))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap();
    let direct = cluster.api_address(0).unwrap();
    let mut client =
        ployz::connect::connect(Path::new("/missing-ployz-test-config"), Some(&direct), None)
            .await
            .unwrap();

    cli(&direct, &["ingress", "deploy"]);
    let ingress_id = wait_service(&mut client, "ingress", 1).await;
    let ingress = wait_running(&mut client, &ingress_id, 1).await.remove(0);
    let ingress_container = ingress.container_id;

    let api_id = ServiceId::random();
    let api: ResolvedServiceSpec = serde_json::from_value(serde_json::json!({
        "service_id": api_id,
        "name": "api",
        "mode": { "mode": "replicated", "replicas": 1 },
        "container": {
            "image": "alpine:3.23.3",
            "command": ["sh", "-c", "while true; do printf 'HTTP/1.1 200 OK\\r\\nContent-Length: 3\\r\\n\\r\\nok\\n' | nc -l -p 8080; done"],
            "pull_policy": "missing"
        },
        "ports": [{
            "mode": "ingress",
            "hostname": { "kind": "explicit", "hostname": "envoy.test" },
            "load_balancer_port": 80,
            "container_port": 8080,
            "http_protocol": "http"
        }]
    }))
    .unwrap();
    let api_container = create_and_start(&mut client, &machine, api).await;
    let observation = wait_running(&mut client, &api_id, 1).await.remove(0);
    let address = observation.address.unwrap();

    let serving = wait_config(&mut client, &machine, |config| {
        config.contains(&format!("address: {}", address.0))
            && config.contains("timeout: 60s")
            && config.contains("connect_timeout: 5s")
    })
    .await;
    assert_eq!(request(&cluster, "envoy.test"), (200, "ok\n".into()));
    assert_ne!(request(&cluster, "missing.test").0, 200);

    change_container(
        &mut client,
        machine.id,
        api_container,
        ContainerAction::Stop,
        Some(0),
    )
    .await;
    let empty = wait_config(&mut client, &machine, |config| {
        config.contains("address: 127.0.0.1")
            && config.contains("port_value: 1")
            && !config.contains(&format!("address: {}", address.0))
    })
    .await;
    assert_ne!(envoy_digest(&serving), envoy_digest(&empty));
    assert_eq!(request(&cluster, "envoy.test").0, 503);
    let still = wait_running(&mut client, &ingress_id, 1).await.remove(0);
    assert_eq!(still.container_id, ingress_container);

    change_container(
        &mut client,
        machine.id,
        api_container,
        ContainerAction::Start,
        None,
    )
    .await;
    let restored = wait_config(&mut client, &machine, |config| {
        config.contains(&format!("address: {}", address.0))
    })
    .await;
    assert_ne!(envoy_digest(&empty), envoy_digest(&restored));
    assert_eq!(request(&cluster, "envoy.test"), (200, "ok\n".into()));
    let after = wait_running(&mut client, &ingress_id, 1).await.remove(0);
    assert_eq!(after.container_id, ingress_container);

    for evidence in [
        "phase=\"validation\" outcome=\"accepted\"",
        "phase=\"activation\" outcome=\"activated\"",
    ] {
        wait_log_count(&cluster, evidence, 1).await;
    }
}

fn envoy_digest(config: &str) -> &str {
    config
        .split_once("projection_digest: ")
        .and_then(|(_, rest)| rest.split(['\n', ' ']).next())
        .expect("rendered Envoy config carries its projection digest")
}
