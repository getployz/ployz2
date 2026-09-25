//! Façade tests for Cloud session `runtime.watch`.

use std::time::Duration;

use ployz::sdk;
use ployz_core::{
    CapabilityName, ContractDescription, DESCRIBE_CONTRACT_CAPABILITY, DockerVolume,
    DockerVolumeId, DockerVolumeName, MACHINE_STORAGE_OBSERVATION_CAPABILITY, MachineId,
    MachineStorageObservation, OpaquePayload, PROTOCOL_MAJOR, RUNTIME_WATCH_CAPABILITY,
    RUNTIME_WATCH_MESSAGE_SIZE_LIMIT, RpcErrorCode, RuntimeWatchFrame,
};
use tokio::time::timeout;
use tonic::Status;

use super::sdk::advertised_description;
use super::support::{DescribeOutcome, DiscoveryService};
use super::unix_session::{self, FakeMachine, UnixSession};

const FROZEN_FRAME: &str =
    include_str!("../../../ployz-core/tests/fixtures/runtime_watch_frame.json");

#[tokio::test]
async fn missing_watch_capability_is_unsupported_and_never_polls_list_rpcs() {
    let description = advertised_description();
    let session = UnixSession::start().await;
    let service = DiscoveryService::new(description.clone());
    let _machine = session
        .spawn_machine(description.machine_id, service.clone())
        .await;
    let client = connect(&session.directory, description.machine_id.as_str()).await;

    let error = match client.watch().await {
        Ok(_) => panic!("Watch must fail when the capability is absent"),
        Err(error) => error,
    };

    assert_eq!(error.code, RpcErrorCode::Unsupported);
    assert!(error.message.contains(RUNTIME_WATCH_CAPABILITY));
    assert_eq!(
        service
            .watch_opens
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_no_list_rpc(&service);
    assert!(
        client
            .about()
            .await
            .unwrap()
            .supports(DESCRIBE_CONTRACT_CAPABILITY)
    );
}

#[tokio::test]
async fn watch_negotiates_gzip() {
    let (client, service, _session, _machine) = watching_session().await;
    let _watch = client.watch().await.unwrap();

    assert!(
        service
            .watch_accepts_gzip
            .load(std::sync::atomic::Ordering::SeqCst)
    );
}

#[tokio::test]
async fn watch_enriches_machine_storage_only_when_the_target_advertises_it() {
    for advertised in [true, false] {
        let description = if advertised {
            storage_watch_description()
        } else {
            watch_description()
        };
        let machine_id = description.machine_id;
        let session = UnixSession::start().await;
        let service = DiscoveryService::new(description);
        service.push_watch_frame(frozen_frame());
        let _machine = session.spawn_machine(machine_id, service).await;
        let client = connect(&session.directory, machine_id.as_str()).await;
        let watch = client.watch().await.unwrap();

        let frame = next_frame(&watch).await;

        assert_eq!(
            frame.machines.first().and_then(|machine| machine.storage),
            advertised.then_some(MachineStorageObservation::Ready)
        );
    }
}

#[tokio::test]
async fn cancel_interrupts_storage_enrichment() {
    let description = storage_watch_description();
    let session = UnixSession::start().await;
    let service = DiscoveryService::new(description.clone());
    service.push_watch_frame(frozen_frame());
    let _machine = session
        .spawn_machine(description.machine_id, service.clone())
        .await;
    let client = connect(&session.directory, description.machine_id.as_str()).await;
    let watch = client.watch().await.unwrap();
    let (received, held) = tokio::sync::oneshot::channel();
    service
        .describe_outcomes
        .lock()
        .unwrap()
        .push_back(DescribeOutcome::Hang(received));
    let waiting = watch.next();
    tokio::pin!(waiting);
    tokio::select! {
        result = &mut waiting => panic!("held enrichment completed: {result:?}"),
        result = timeout(Duration::from_secs(2), held) => {
            result.expect("enrichment request reached the server").unwrap();
        }
    }

    watch.cancel();

    assert_eq!(
        timeout(Duration::from_secs(1), waiting)
            .await
            .expect("cancel must interrupt storage enrichment")
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn storage_enrichment_has_a_short_overall_budget() {
    let description = storage_watch_description();
    let session = UnixSession::start().await;
    let service = DiscoveryService::new(description.clone());
    service.push_watch_frame(frozen_frame());
    let _machine = session
        .spawn_machine(description.machine_id, service.clone())
        .await;
    let client = connect(&session.directory, description.machine_id.as_str()).await;
    let watch = client.watch().await.unwrap();
    let (received, held) = tokio::sync::oneshot::channel();
    service
        .describe_outcomes
        .lock()
        .unwrap()
        .push_back(DescribeOutcome::Hang(received));

    let (frame, ()) = timeout(Duration::from_secs(4), async {
        tokio::join!(watch.next(), async {
            held.await.expect("enrichment request reached the server");
            // Advance only once the held request has armed the overall budget.
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(3)).await;
            tokio::time::resume();
        })
    })
    .await
    .expect("storage enrichment must have a short overall budget");
    let frame = frame.unwrap().unwrap();

    assert_eq!(
        frame.machines.first().and_then(|machine| machine.storage),
        None
    );
}

#[tokio::test]
async fn changed_frame_yields_another_complete_frame() {
    let (client, service, _session, _machine) = watching_session().await;
    let first = frozen_frame();
    let second = frame_with_extra_volume(&first);
    service.push_watch_frame(first.clone());
    let watch = client.watch().await.unwrap();
    assert_eq!(next_frame(&watch).await, first);

    service.push_watch_frame(second.clone());

    let changed = next_frame(&watch).await;
    assert_eq!(changed, second);
    assert_ne!(changed, first);
    assert_no_list_rpc(&service);
}

#[tokio::test]
async fn abort_ends_only_that_watch_and_leaves_the_client_usable() {
    let (client, service, _session, _machine) = watching_session().await;
    service.push_watch_frame(frozen_frame());
    let watch = client.watch().await.unwrap();
    let _first = next_frame(&watch).await;
    let waiting = watch.next();
    tokio::pin!(waiting);
    assert!(
        timeout(Duration::from_millis(50), &mut waiting)
            .await
            .is_err()
    );

    watch.cancel();

    assert_eq!(
        timeout(Duration::from_secs(1), waiting)
            .await
            .expect("cancel must unblock next()")
            .unwrap(),
        None
    );
    wait_watch_rpc_dropped(&service).await;
    assert!(
        client
            .about()
            .await
            .unwrap()
            .supports(DESCRIBE_CONTRACT_CAPABILITY)
    );

    service.push_watch_frame(frozen_frame());
    let again = client.watch().await.unwrap();
    let _ = next_frame(&again).await;
    again.cancel();
    wait_watch_rpc_dropped(&service).await;
    assert_eq!(
        service
            .watch_opens
            .load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert_no_list_rpc(&service);
}

#[tokio::test]
async fn watch_termination_is_a_typed_error_asking_the_caller_to_reconnect() {
    enum End {
        Fail(Status),
        Closed,
        LostConnection,
    }
    for (end, message) in [
        (
            End::Fail(Status::unavailable("store closed")),
            Some("store closed"),
        ),
        (End::Closed, None),
        (End::LostConnection, None),
        (End::Fail(Status::cancelled("daemon cancelled Watch")), None),
    ] {
        let (client, service, _session, machine) = watching_session().await;
        service.push_watch_frame(frozen_frame());
        let watch = client.watch().await.unwrap();
        let _first = next_frame(&watch).await;

        match end {
            End::Fail(status) => service.fail_watch(status),
            End::Closed => service.end_watch(),
            End::LostConnection => machine.disconnect(),
        }

        let error = timeout(Duration::from_secs(2), watch.next())
            .await
            .expect("termination must end the iterable")
            .expect_err("termination is a typed error");
        assert_eq!(error.code, RpcErrorCode::Unavailable, "{error:?}");
        if let Some(message) = message {
            assert!(error.message.contains(message), "{error:?}");
        }
    }
}

#[tokio::test]
async fn node_watch_decodes_frames_above_tonics_default() {
    let description = storage_watch_description();
    let session = UnixSession::start().await;
    let service = DiscoveryService::new(description.clone());
    let mut frame = frozen_frame();
    frame.observed_at = "x".repeat(4 * 1024 * 1024);
    service.emit_watch_frame_on_open(frame);
    let _machine = session
        .spawn_machine(description.machine_id, service.clone())
        .await;
    session
        .assert_sdk_script("node_watch.js", description.machine_id, &[])
        .await;
    assert_eq!(
        service
            .watch_opens
            .load(std::sync::atomic::Ordering::SeqCst),
        3
    );
    assert_no_list_rpc(&service);
}

#[tokio::test]
async fn frame_above_ceiling_errors_without_closing_session() {
    let (client, service, _session, _machine) = watching_session().await;
    let mut payload = OpaquePayload::from_json(&frozen_frame()).unwrap();
    // Trailing whitespace keeps valid frame JSON without serializing a 64 MiB string.
    payload
        .json
        .resize(RUNTIME_WATCH_MESSAGE_SIZE_LIMIT + 1, b' ');
    service.push_watch_payload(payload);
    let watch = client.watch().await.unwrap();

    let error = timeout(Duration::from_secs(10), watch.next())
        .await
        .expect("Watch error")
        .expect_err("oversized Watch frame must fail");

    assert_eq!(error.code, RpcErrorCode::Internal);
    assert!(
        error.message.contains("message length too large"),
        "{}",
        error.message
    );
    assert!(
        error
            .message
            .contains(&RUNTIME_WATCH_MESSAGE_SIZE_LIMIT.to_string())
    );
    assert!(
        client
            .about()
            .await
            .unwrap()
            .supports(DESCRIBE_CONTRACT_CAPABILITY)
    );
}

async fn watching_session() -> (sdk::Session, DiscoveryService, UnixSession, FakeMachine) {
    let description = watch_description();
    let session = UnixSession::start().await;
    let service = DiscoveryService::new(description.clone());
    let machine = session
        .spawn_machine(description.machine_id, service.clone())
        .await;
    let client = connect(&session.directory, description.machine_id.as_str()).await;
    (client, service, session, machine)
}

async fn connect(url: &str, machine_id: &str) -> sdk::Session {
    timeout(
        Duration::from_secs(5),
        unix_session::connect(url, machine_id),
    )
    .await
    .expect("connect must not hang")
    .unwrap()
}

async fn next_frame(watch: &sdk::Watch) -> RuntimeWatchFrame {
    timeout(Duration::from_secs(2), watch.next())
        .await
        .expect("Watch frame")
        .expect("Watch result")
        .expect("Watch item")
}

async fn wait_watch_rpc_dropped(service: &DiscoveryService) {
    timeout(Duration::from_secs(2), async {
        loop {
            if service.live_watch_senders() == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("cancel must drop the Watch RPC without another next()");
}

fn watch_description() -> ContractDescription {
    ContractDescription {
        machine_id: MachineId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        protocol_major: PROTOCOL_MAJOR,
        daemon_version: "do-not-branch-on-me".into(),
        capabilities: [
            CapabilityName::parse(DESCRIBE_CONTRACT_CAPABILITY)
                .expect("catalogued capability names are valid"),
            CapabilityName::parse(RUNTIME_WATCH_CAPABILITY)
                .expect("catalogued capability names are valid"),
        ]
        .into(),
    }
}

fn storage_watch_description() -> ContractDescription {
    let mut description = watch_description();
    description.capabilities.insert(
        CapabilityName::parse(MACHINE_STORAGE_OBSERVATION_CAPABILITY)
            .expect("catalogued capability names are valid"),
    );
    description
}

fn frozen_frame() -> RuntimeWatchFrame {
    serde_json::from_str(FROZEN_FRAME).unwrap()
}

fn frame_with_extra_volume(frame: &RuntimeWatchFrame) -> RuntimeWatchFrame {
    let mut changed = frame.clone();
    changed.volumes.push(DockerVolume {
        id: DockerVolumeId {
            machine_id: MachineId::parse("0123456789abcdef0123456789abcdef").unwrap(),
            name: DockerVolumeName::parse("logs").unwrap(),
        },
        options: Default::default(),
        labels: Default::default(),
        storage: ployz_core::DockerVolumeStorageObservation::Plain {
            driver: "local".into(),
        },
    });
    changed
}

fn assert_no_list_rpc(service: &DiscoveryService) {
    assert_eq!(
        service
            .list_rpc_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn sdk_about_and_watch_retry_transient_contract_read() {
    let (client, service, _session, _machine) = watching_session().await;
    service
        .describe_outcomes
        .lock()
        .unwrap()
        .push_back(DescribeOutcome::Status(Status::unavailable("lost read")));
    client.about().await.expect("safe read redials");
    service
        .describe_outcomes
        .lock()
        .unwrap()
        .push_back(DescribeOutcome::Status(Status::unavailable(
            "lost watch contract read",
        )));
    let watch = client
        .watch()
        .await
        .expect("Watch uses its redialed client");
    watch.cancel();
    client.close().await;
}
