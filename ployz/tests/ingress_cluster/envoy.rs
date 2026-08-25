//! Envoy founding tracer against the privileged Layer 3 cluster.

use std::{path::Path, process, time::Duration};

use ployz_core::{IngressProxyBackend, InspectRequest, op};
use ployz_testkit::{Cluster, ClusterPlan};

use super::{cli, wait_running, wait_service};

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
