//! Façade tests for Cloud session Machine removal with named Data Loss.

use std::{collections::BTreeMap, time::Duration};

use ployz::sdk;
use ployz_core::{
    ContractDescription, DataLoss, DockerVolumeId, DockerVolumeName, MachineId, MachineObservation,
    RpcErrorCode, UnconfirmedDataLoss,
};
use tokio::time::timeout;

use super::support::{DiscoveryService, confirmation, connected_client, machine};
use super::support::{docker_volume, machine_named};
use super::unix_session::{self, UnixSession};

#[tokio::test]
async fn remove_machine_destroys_a_peer_after_named_data_loss_confirmation() {
    let (client, worker, empty, service, _session, _machine) = removal_session().await;
    let observed = client
        .data_loss_if_machine_removed(worker.name.as_str())
        .await
        .unwrap();
    assert_eq!(observed.data_loss.len(), 2);
    let reviewed_confirmation = confirmation(observed.data_loss);

    let removed = client
        .remove_machine(worker.name.as_str(), &reviewed_confirmation)
        .await
        .unwrap();
    assert!(removed.reset_warning.is_none());
    assert_eq!(
        service.reset_machines.lock().unwrap().as_slice(),
        &[worker.id]
    );
    assert_eq!(
        service.removed_machines.lock().unwrap().as_slice(),
        &[worker.id]
    );

    let empty_confirmation = confirmation(Vec::<DataLoss>::new());
    let none = client
        .remove_machine(empty.name.as_str(), &empty_confirmation)
        .await
        .unwrap();
    assert!(none.reset_warning.is_none());
}

#[tokio::test]
async fn remove_machine_fails_when_fresh_data_loss_is_unconfirmed() {
    let (client, worker, _empty, service, _session, _machine) = removal_session().await;
    let confirmation = confirmation(Vec::<DataLoss>::new());

    let error = client
        .remove_machine(worker.name.as_str(), &confirmation)
        .await
        .unwrap_err();
    assert_eq!(error.code, RpcErrorCode::InvalidArgument);
    assert!(error.message.contains("data"));
    assert!(error.message.contains("logs"));
    let missing: UnconfirmedDataLoss = serde_json::from_value(error.details).unwrap();
    assert_eq!(
        missing.missing,
        [volume(worker.id, "data"), volume(worker.id, "logs"),]
    );
    assert!(service.reset_machines.lock().unwrap().is_empty());
    assert!(service.removed_machines.lock().unwrap().is_empty());
}

#[tokio::test]
async fn remove_machine_refuses_the_current_entry_while_another_is_visible() {
    let (client, _worker, _empty, service, _session, _machine) = removal_session().await;
    let entry = client.about().await.unwrap().machine_id;
    let confirmation = confirmation(Vec::<DataLoss>::new());

    let error = client
        .remove_machine(entry.as_str(), &confirmation)
        .await
        .unwrap_err();
    assert_eq!(error.code, RpcErrorCode::InvalidArgument);
    assert!(
        error.message.contains(
            "the current entry Machine cannot be removed while another Machine is visible"
        )
    );
    assert!(service.reset_machines.lock().unwrap().is_empty());
    assert!(service.removed_machines.lock().unwrap().is_empty());
}

#[tokio::test]
async fn remove_machine_reports_a_failed_reset_instead_of_swallowing_it() {
    let (description, worker, _empty, service) = removal_cluster();
    *service.reset_warning.lock().unwrap() = Some("replicated delete failed".into());
    let session = UnixSession::start().await;
    let spawned = session
        .spawn_machine(description.machine_id, service.clone())
        .await;
    let client = timeout(
        Duration::from_secs(5),
        unix_session::connect(&session.directory, description.machine_id.as_str()),
    )
    .await
    .expect("connect must not hang")
    .unwrap();
    let confirmation = confirmation([
        volume(worker.machine.id, "data"),
        volume(worker.machine.id, "logs"),
    ]);

    let removed = client
        .remove_machine(worker.machine.name.as_str(), &confirmation)
        .await
        .unwrap();
    assert_eq!(
        removed.reset_warning.as_deref(),
        Some("replicated delete failed")
    );
    assert_eq!(
        service.removed_machines.lock().unwrap().as_slice(),
        &[worker.machine.id]
    );
    drop(spawned);
}

#[tokio::test]
async fn last_machine_removal_refuses_only_when_a_management_client_holds_a_key() {
    let cloud = "Delete the Cluster from Cloud instead";
    let non_cloud = "this is the last Machine in the Cluster and it is still managed by `cli` and `ops`; \
         removing it would leave `cli` and `ops` managing a Cluster that no longer exists. \
         Disconnect `cli` and `ops` from this Machine first.";
    // (reset the Machine, key holders, expected refusal, is the message exact)
    for (reset, holders, refusal, exact) in [
        (true, &["cloud"][..], Some(cloud), false),
        (false, &["cloud"], Some(cloud), false),
        (false, &["cli", "ops"], Some(non_cloud), true),
        (false, &[], None, false),
        (true, &[], None, false),
    ] {
        let (_description, entry, service) = last_machine_cluster();
        hold_keys(&service, holders);
        let (mut client, server, _) = connected_client(service.clone()).await;
        let target = ployz_core::MachineTarget::from(&entry.machine.id);

        let result = if reset {
            client
                .remove_machine(&target, &confirmation(Vec::<DataLoss>::new()))
                .await
                .map(|removed| assert!(removed.reset_warning.is_none()))
        } else {
            client.remove_machine_membership(&target).await
        };

        let reset_machines = service.reset_machines.lock().unwrap().clone();
        let removed_machines = service.removed_machines.lock().unwrap().clone();
        let case = format!("reset={reset} holders={holders:?}");
        match refusal {
            Some(message) => {
                let error = result.unwrap_err();
                assert_eq!(error.code, RpcErrorCode::InvalidArgument, "{case}");
                if exact {
                    assert_eq!(error.message, message, "{case}");
                } else {
                    assert!(error.message.contains(message), "{case}: {error:?}");
                }
                assert!(reset_machines.is_empty(), "{case}");
                assert!(removed_machines.is_empty(), "{case}");
            }
            // A clean reset of the only Machine needs no separate shared-row removal.
            None if reset => {
                result.unwrap();
                assert_eq!(reset_machines, [entry.machine.id], "{case}");
                assert!(removed_machines.is_empty(), "{case}");
            }
            None => {
                result.unwrap();
                assert!(reset_machines.is_empty(), "{case}");
                assert_eq!(removed_machines, [entry.machine.id], "{case}");
            }
        }
        server.abort();
    }
}

fn hold_keys(service: &DiscoveryService, labels: &[&str]) {
    *service.management_clients.lock().unwrap() = labels
        .iter()
        .map(|label| ployz_core::ManagementClientLabel::parse(*label).unwrap())
        .collect();
}

fn last_machine_cluster() -> (ContractDescription, MachineObservation, DiscoveryService) {
    let description = super::sdk::advertised_description();
    let entry = machine_named(&description.machine_id, "entry");
    let mut service = DiscoveryService::new(description.clone());
    service.machines = vec![entry.clone()];
    *service.listed_volumes.lock().unwrap() = BTreeMap::from([(entry.machine.id, vec![])]);
    (description, entry, service)
}

#[tokio::test]
async fn node_remove_machine_covers_volumes_and_unconfirmed_missing_names() {
    let (description, worker, empty, service) = removal_cluster();
    let session = UnixSession::start().await;
    let _machine = session.spawn_machine(description.machine_id, service).await;
    session
        .assert_sdk_script(
            "node_remove_machine.js",
            description.machine_id,
            &[
                ("PLOYZ_WORKER_MACHINE", worker.machine.name.as_str()),
                ("PLOYZ_EMPTY_MACHINE", empty.machine.name.as_str()),
            ],
        )
        .await;
}

async fn removal_session() -> (
    sdk::Session,
    ployz_core::Machine,
    ployz_core::Machine,
    DiscoveryService,
    UnixSession,
    super::unix_session::FakeMachine,
) {
    let (description, worker, empty, service) = removal_cluster();
    let session = UnixSession::start().await;
    let spawned = session
        .spawn_machine(description.machine_id, service.clone())
        .await;
    let client = timeout(
        Duration::from_secs(5),
        unix_session::connect(&session.directory, description.machine_id.as_str()),
    )
    .await
    .expect("connect must not hang")
    .unwrap();
    (
        client,
        worker.machine,
        empty.machine,
        service,
        session,
        spawned,
    )
}

fn removal_cluster() -> (
    ContractDescription,
    MachineObservation,
    MachineObservation,
    DiscoveryService,
) {
    let description = super::sdk::advertised_description();
    let entry = machine_named(&description.machine_id, "entry");
    let worker = machine('c', "worker");
    let empty = machine('e', "empty");
    let mut service = DiscoveryService::new(description.clone());
    service.machines = vec![entry, worker.clone(), empty.clone()];
    *service.listed_volumes.lock().unwrap() = BTreeMap::from([
        (
            worker.machine.id,
            vec![
                docker_volume(worker.machine.id, "data"),
                docker_volume(worker.machine.id, "logs"),
            ],
        ),
        (empty.machine.id, vec![]),
        (description.machine_id, vec![]),
    ]);
    (description, worker, empty, service)
}

fn volume(machine_id: MachineId, name: &str) -> DataLoss {
    DataLoss::DockerVolume {
        id: DockerVolumeId {
            machine_id,
            name: DockerVolumeName::parse(name).unwrap(),
        },
    }
}
