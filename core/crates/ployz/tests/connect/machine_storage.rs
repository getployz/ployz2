use std::sync::atomic::Ordering;

use ployz_core::{
    CapabilityName, ContractDescription, MACHINE_STORAGE_OBSERVATION_CAPABILITY, MachineId,
    PROTOCOL_MAJOR,
};
use serde_json::Value;

use super::{
    run_ployz,
    support::{DiscoveryService, connected_client, machine, serve_discovery},
};

fn storage_service(advertised: bool) -> DiscoveryService {
    DiscoveryService::new(ContractDescription {
        machine_id: MachineId::random(),
        protocol_major: PROTOCOL_MAJOR,
        daemon_version: "test".into(),
        capabilities: if advertised {
            [CapabilityName::parse(MACHINE_STORAGE_OBSERVATION_CAPABILITY).unwrap()].into()
        } else {
            Default::default()
        },
    })
}

#[tokio::test]
async fn machines_returns_raw_list_machines_observations_without_storage_fanout() {
    let service = storage_service(true);
    let (mut client, server, _) = connected_client(service.clone()).await;

    let observed = client.machines().await.unwrap();

    assert_eq!(observed, vec![machine('a', "one")]);
    assert_eq!(service.inspect_calls.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn machine_ls_observes_storage_only_when_the_target_advertises_it() {
    for advertised in [true, false] {
        let mut service = storage_service(advertised);
        service.storage = ployz_core::MachineStorageObservation::Pool {
            size_bytes: std::num::NonZeroU64::new(4_294_967_296).unwrap(),
            used_bytes: 3_865_470_566,
            free_bytes: 429_496_730,
        };
        let (address, server) = serve_discovery(service).await;

        let output = run_ployz(address, &["machine", "ls", "--output", "json"]).await;

        assert!(output.status.success(), "{output:?}");
        let observed: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            observed.pointer("/0/storage"),
            Some(&if advertised {
                serde_json::json!({
                    "state": "pool",
                    "size_bytes": 4_294_967_296_u64,
                    "used_bytes": 3_865_470_566_u64,
                    "free_bytes": 429_496_730_u64,
                })
            } else {
                Value::Null
            })
        );
        server.abort();
    }
}

#[tokio::test]
async fn machine_ls_warns_without_failing_only_when_a_daemon_version_differs() {
    for (daemon_version, expected_stderr) in [
        (
            "0.0.0-old",
            format!(
                "WARNING: 1 Machine runs a daemon version different from CLI {}.\n",
                env!("CARGO_PKG_VERSION")
            ),
        ),
        (env!("CARGO_PKG_VERSION"), String::new()),
    ] {
        let mut service = storage_service(true);
        service
            .machines
            .first_mut()
            .unwrap()
            .machine
            .runtime
            .daemon_version = daemon_version.into();
        let (address, server) = serve_discovery(service).await;

        let output = run_ployz(address, &["machine", "ls"]).await;

        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            expected_stderr,
            "{daemon_version}"
        );
        server.abort();
    }
}
