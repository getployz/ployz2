use std::{
    collections::HashSet,
    net::Ipv6Addr,
    sync::{Arc, Mutex},
};

use ployz_core::{
    AdvertisedEndpoint, JoinRequest, LocalMachinePhase, MachineId, MachineRuntime,
    ManagementAddress, RegisterRequest, RpcErrorCode, WireGuardPublicKey,
};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;

use super::{LocalMachine, LocalMachineError, LocalMachineStore, StoreError};
use crate::{
    corrosion::{AdminClient, fake_cluster},
    machine_api::{MachineApi, MachineService},
};

mod harness;
use harness::*;

#[tokio::test]
async fn register_rejects_an_uninitialized_machine() {
    let data_dir = std::env::temp_dir().join(format!(
        "ployzd-register-errors-{}",
        ployz_core::MachineId::random()
    ));
    let local = LocalMachine::new(
        Arc::new(Mutex::new(LocalMachineStore::open(&data_dir).unwrap())),
        watch::channel(false).0,
    );
    let empty = local
        .register(RegisterRequest {
            advertised_endpoints: Vec::new(),
            ..request("peer", WireGuardPublicKey([1; 32]))
        })
        .await
        .unwrap_err();
    assert!(matches!(empty, LocalMachineError::NotParticipating));
    let uninitialized = local
        .register(request("peer", WireGuardPublicKey([1; 32])))
        .await
        .unwrap_err();
    assert!(matches!(uninitialized, LocalMachineError::NotParticipating));
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn registration_target_requires_a_cluster_store() {
    let data_dir = std::env::temp_dir().join(format!(
        "ployzd-register-target-errors-{}",
        MachineId::random()
    ));
    let local = LocalMachine::new(
        Arc::new(Mutex::new(LocalMachineStore::open(&data_dir).unwrap())),
        watch::channel(false).0,
    );

    let error = local.registration_target().await.unwrap_err();

    assert!(matches!(error, LocalMachineError::ClusterStoreUnavailable));
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn register_assigns_a_free_subnet_publishes_and_rejects_duplicates() {
    let (local, replicated, founder, data_dir, server) = participating().await;
    let missing = local
        .register(RegisterRequest {
            advertised_endpoints: Vec::new(),
            ..request("peer", WireGuardPublicKey([1; 32]))
        })
        .await
        .unwrap_err();
    assert!(matches!(
        missing,
        LocalMachineError::Store(StoreError::MissingEndpoints)
    ));
    let registered = local
        .register(request("peer", WireGuardPublicKey([1; 32])))
        .await
        .unwrap();
    assert_eq!(
        registered.assigned_machine.subnet,
        "10.210.1.0/24".parse().unwrap()
    );
    assert_eq!(registered.visible_peers, vec![founder]);
    assert_eq!(
        replicated
            .machine(registered.assigned_machine.id.as_str())
            .await
            .unwrap()
            .as_ref(),
        Some(&registered.assigned_machine)
    );

    let duplicate_name = local
        .register(request("peer", WireGuardPublicKey([2; 32])))
        .await
        .unwrap_err();
    assert!(matches!(duplicate_name, LocalMachineError::NameTaken));
    let duplicate_key = local
        .register(request("other", WireGuardPublicKey([1; 32])))
        .await
        .unwrap_err();
    assert!(matches!(duplicate_key, LocalMachineError::KeyAlreadyNamed));

    server.abort();
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn register_rpc_exact_replay_returns_the_original_joinable_assignment() {
    let (data_dir, store, founder) = open_store("ployzd-register-replay");
    let (replicated, server) = fake_cluster::store().await;
    replicated.publish_local_machine(&founder).await.unwrap();
    let service = machine_service(store, replicated.clone(), None);

    let target_dir = std::env::temp_dir().join(format!(
        "ployzd-register-replay-target-{}",
        MachineId::random()
    ));
    let target_store = Arc::new(Mutex::new(LocalMachineStore::open(&target_dir).unwrap()));
    let public_key = target_store
        .lock()
        .unwrap()
        .record()
        .wireguard_private_key
        .public_key();
    let identity = request("joiner", public_key);

    let first = rpc_register(&service, identity.clone(), false)
        .await
        .unwrap();
    let replay = rpc_register(&service, identity, false).await.unwrap();

    assert_eq!(replay.assigned_machine.id, first.assigned_machine.id);
    assert_eq!(
        replay.assigned_machine.subnet,
        first.assigned_machine.subnet
    );
    assert_eq!(replicated.machines().await.unwrap().observations.len(), 2);

    let target = LocalMachine::new(target_store, watch::channel(false).0);
    target
        .join(JoinRequest {
            registration: replay,
            wireguard_mtu: None,
            cloud_pairing: None,
        })
        .unwrap();
    assert_eq!(target.record().unwrap().phase(), LocalMachinePhase::Joining);

    let conflict = rpc_register(&service, request("other", public_key), false)
        .await
        .unwrap_err();
    assert_eq!(conflict.code, RpcErrorCode::Conflict);

    server.abort();
    drop(service);
    drop(target);
    let _ = std::fs::remove_dir_all(data_dir);
    let _ = std::fs::remove_dir_all(target_dir);
}

#[tokio::test]
async fn register_does_not_reconstruct_membership_while_joining() {
    let (local, replicated, _founder, data_dir, server) = participating().await;
    let joiner_dir =
        std::env::temp_dir().join(format!("ployzd-register-joining-{}", MachineId::random()));
    let joiner_store = Arc::new(Mutex::new(LocalMachineStore::open(&joiner_dir).unwrap()));
    let public_key = joiner_store
        .lock()
        .unwrap()
        .record()
        .wireguard_private_key
        .public_key();
    let registered = local.register(request("peer", public_key)).await.unwrap();
    let joiner = LocalMachine::new(joiner_store, watch::channel(false).0).with_cluster(Some((
        replicated.clone(),
        AdminClient::new("/no/such/ployz-admin.sock"),
    )));
    joiner
        .join(JoinRequest {
            registration: registered,
            wireguard_mtu: None,
            cloud_pairing: None,
        })
        .unwrap();
    assert_eq!(joiner.record().unwrap().phase(), LocalMachinePhase::Joining);

    let error = joiner
        .register(request("peer", public_key))
        .await
        .unwrap_err();
    assert!(matches!(error, LocalMachineError::NotParticipating));

    server.abort();
    drop(local);
    drop(joiner);
    let _ = std::fs::remove_dir_all(data_dir);
    let _ = std::fs::remove_dir_all(joiner_dir);
}

#[tokio::test]
async fn register_returns_the_committed_row_after_join_runtime_and_endpoint_drift() {
    let (local, replicated, _founder, data_dir, server) = participating().await;
    let key = WireGuardPublicKey([1; 32]);
    let registered = local.register(request("peer", key)).await.unwrap();
    let assigned_id = registered.assigned_machine.id;
    let mut drifted = registered.assigned_machine.clone();
    drifted.runtime = MachineRuntime {
        daemon_version: "joined".into(),
        ..MachineRuntime::default()
    };
    drifted.public_ip = Some("198.51.100.9".parse().unwrap());
    drifted.advertised_endpoints = vec![AdvertisedEndpoint("198.51.100.9:51820".parse().unwrap())];
    replicated.publish_local_machine(&drifted).await.unwrap();

    let mut replay = request("peer", key);
    replay.public_ip = None;
    replay.runtime = MachineRuntime::default();
    let replayed = local.register(replay).await.unwrap();

    assert_eq!(replayed.assigned_machine.id, assigned_id);
    assert_eq!(replayed.assigned_machine.subnet, drifted.subnet);
    assert_eq!(replayed.assigned_machine.runtime.daemon_version, "joined");
    assert_eq!(
        replayed.assigned_machine.advertised_endpoints,
        drifted.advertised_endpoints
    );
    let stored = replicated
        .machine(assigned_id.as_str())
        .await
        .unwrap()
        .expect("committed Machine remains");
    assert_eq!(stored, drifted);
    assert_eq!(replicated.machines().await.unwrap().observations.len(), 2);

    let renamed = local.register(request("other", key)).await.unwrap_err();
    assert!(matches!(renamed, LocalMachineError::KeyAlreadyNamed));

    server.abort();
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn register_returns_the_committed_row_when_advertised_endpoints_are_empty() {
    let (local, replicated, _founder, data_dir, server) = participating().await;
    let key = WireGuardPublicKey([1; 32]);
    let registered = local.register(request("peer", key)).await.unwrap();
    let assigned_id = registered.assigned_machine.id;
    let committed = registered.assigned_machine.clone();

    let mut replay = request("peer", key);
    replay.advertised_endpoints = Vec::new();
    let replayed = local.register(replay).await.unwrap();

    assert_eq!(replayed.assigned_machine.id, assigned_id);
    assert_eq!(
        replayed.assigned_machine.advertised_endpoints,
        committed.advertised_endpoints
    );
    let stored = replicated
        .machine(assigned_id.as_str())
        .await
        .unwrap()
        .expect("committed Machine remains");
    assert_eq!(stored, committed);
    assert_eq!(replicated.machines().await.unwrap().observations.len(), 2);

    server.abort();
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn concurrent_registers_fill_a_200_machine_cluster_without_subnet_collisions() {
    let (local, replicated, _founder, data_dir, server) = participating().await;
    let mut joins = tokio::task::JoinSet::new();
    for seed in 1_u8..200 {
        let local = local.clone();
        joins.spawn(async move {
            local
                .register(request(
                    &format!("peer-{seed}"),
                    WireGuardPublicKey([seed; 32]),
                ))
                .await
                .unwrap()
                .assigned_machine
                .subnet
        });
    }
    let mut subnets = HashSet::new();
    while let Some(result) = joins.join_next().await {
        assert!(subnets.insert(result.unwrap()));
    }
    assert_eq!(subnets.len(), 199);
    assert_eq!(replicated.machines().await.unwrap().observations.len(), 200);

    server.abort();
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn register_does_not_allocate_when_another_machine_is_the_target() {
    let (local, replicated, _founder, data_dir, server) = participating().await;
    let target = unreachable_machine(MachineId::parse("0".repeat(32)).unwrap());
    replicated.publish_local_machine(&target).await.unwrap();

    let error = local
        .register(request("peer", WireGuardPublicKey([1; 32])))
        .await
        .unwrap_err();

    assert!(matches!(error, LocalMachineError::NotRegistrationTarget));
    assert_eq!(replicated.machines().await.unwrap().observations.len(), 2);
    server.abort();
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn contact_forwards_register_and_returns_the_target_payload() {
    let (target_dir, target_store, mut reachable) = open_store("ployzd-register-target");
    let (target_replica, target_cluster) = fake_cluster::store().await;
    reachable.management_address = ManagementAddress(Ipv6Addr::LOCALHOST);
    target_replica
        .publish_local_machine(&reachable)
        .await
        .unwrap();
    let listener = TcpListener::bind("[::1]:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(Server::builder().serve_with_incoming(
        MachineApi::from_local(machine_service(target_store, target_replica.clone(), None)),
        TcpListenerStream::new(listener),
    ));

    let (contact_dir, contact_store, _contact) = open_store("ployzd-register-contact");
    let (contact_replica, contact_cluster) = fake_cluster::store().await;
    contact_replica
        .publish_local_machine(&reachable)
        .await
        .unwrap();
    let contact = machine_service(contact_store, contact_replica.clone(), Some(port));

    let registered = rpc_register(
        &contact,
        request("joiner", WireGuardPublicKey([7; 32])),
        false,
    )
    .await
    .unwrap();

    assert_eq!(registered.assigned_machine.name.as_str(), "joiner");
    assert_eq!(
        registered.assigned_machine.subnet,
        "10.210.1.0/24".parse().unwrap()
    );
    assert!(
        target_replica
            .machines()
            .await
            .unwrap()
            .observations
            .iter()
            .any(|machine| machine.name.as_str() == "joiner"),
        "target admits locally"
    );
    assert!(
        contact_replica
            .machines()
            .await
            .unwrap()
            .observations
            .iter()
            .all(|machine| machine.name.as_str() != "joiner"),
        "contact must not allocate locally"
    );
    target_cluster.abort();
    contact_cluster.abort();
    let _ = std::fs::remove_dir_all(target_dir);
    let _ = std::fs::remove_dir_all(contact_dir);
}

#[tokio::test]
async fn forwarded_register_does_not_admit_or_forward_when_another_machine_is_target() {
    let (data_dir, store, local_machine) = open_store("ployzd-register-one-hop");
    let other = MachineId::parse("0".repeat(32)).unwrap();
    let mut named = local_machine;
    named.id = other;
    named.management_address = ManagementAddress(Ipv6Addr::LOCALHOST);
    let (replicated, server) = fake_cluster::store().await;
    replicated.publish_local_machine(&named).await.unwrap();
    let local = machine_service(store, replicated.clone(), Some(1));

    let error = rpc_register(&local, request("joiner", WireGuardPublicKey([7; 32])), true)
        .await
        .unwrap_err();

    assert_eq!(error.message, "this Machine is not the registration target");
    assert!(
        replicated
            .machines()
            .await
            .unwrap()
            .observations
            .iter()
            .all(|machine| machine.name.as_str() != "joiner")
    );
    server.abort();
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn unreachable_registration_target_is_retryable_without_a_subnet() {
    let (data_dir, store, _founder) = open_store("ployzd-register-unreachable");
    let target = unreachable_machine(MachineId::parse("0".repeat(32)).unwrap());
    let (replicated, server) = fake_cluster::store().await;
    replicated.publish_local_machine(&target).await.unwrap();
    let local = machine_service(store, replicated.clone(), Some(1));

    let error = rpc_register(
        &local,
        request("joiner", WireGuardPublicKey([7; 32])),
        false,
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, RpcErrorCode::Unavailable);
    assert_eq!(error.message, "Registration target is unreachable");
    assert!(
        replicated
            .machines()
            .await
            .unwrap()
            .observations
            .iter()
            .all(|machine| machine.name.as_str() != "joiner"),
        "an unreachable target must not assign a Machine Subnet"
    );
    server.abort();
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn forwarded_rpc_metadata_admits_locally_only() {
    let (data_dir, store, founder) = open_store("ployzd-register-metadata");
    let (replicated, server) = fake_cluster::store().await;
    replicated.publish_local_machine(&founder).await.unwrap();
    let service = machine_service(store, replicated.clone(), None);
    let registered = rpc_register(
        &service,
        request("joiner", WireGuardPublicKey([7; 32])),
        true,
    )
    .await
    .unwrap();

    assert_eq!(registered.assigned_machine.name.as_str(), "joiner");
    server.abort();
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn isolation_lock_refuses_admit_when_replica_exceeds_three_and_others_are_uncontactable() {
    let (data_dir, store, founder) = open_store("ployzd-register-isolation-admit");
    let (replicated, server) = fake_cluster::store().await;
    replicated.publish_local_machine(&founder).await.unwrap();
    publish_peers(&replicated, 3).await;
    let (admin_server, admin, admin_root) = serve_membership(&[]).await;
    let local = LocalMachine::new(store, watch::channel(false).0)
        .with_cluster(Some((replicated.clone(), AdminClient::new(&admin))));
    let error = local
        .register(request("joiner", WireGuardPublicKey([1; 32])))
        .await
        .unwrap_err();
    assert!(matches!(error, LocalMachineError::IsolationLocked));
    assert_eq!(replicated.machines().await.unwrap().observations.len(), 4);

    admin_server.abort();
    let _ = std::fs::remove_dir_all(admin_root);
    server.abort();
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn isolation_lock_refuses_forward_when_replica_exceeds_three_and_others_are_uncontactable() {
    let (data_dir, store, _founder) = open_store("ployzd-register-isolation-forward");
    let target = unreachable_machine(MachineId::parse("0".repeat(32)).unwrap());
    let (replicated, server) = fake_cluster::store().await;
    replicated.publish_local_machine(&target).await.unwrap();
    publish_peers(&replicated, 3).await;
    let (admin_server, admin, admin_root) = serve_membership(&[]).await;
    let local = MachineService::with_cluster(
        store,
        watch::channel(false).0,
        Some((replicated.clone(), AdminClient::new(&admin))),
    )
    .with_machine_api_port(1);

    let error = rpc_register(
        &local,
        request("joiner", WireGuardPublicKey([7; 32])),
        false,
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, RpcErrorCode::Unavailable);
    assert_eq!(error.message, "this Machine is isolation-locked");
    assert!(
        replicated
            .machines()
            .await
            .unwrap()
            .observations
            .iter()
            .all(|machine| machine.name.as_str() != "joiner"),
        "isolation must not forward or assign a Machine Subnet"
    );
    admin_server.abort();
    let _ = std::fs::remove_dir_all(admin_root);
    server.abort();
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn isolation_lock_does_not_fire_when_a_peer_is_still_up() {
    let (data_dir, store, founder) = open_store("ployzd-register-isolation-split-admit");
    let (replicated, cluster) = fake_cluster::store().await;
    replicated.publish_local_machine(&founder).await.unwrap();
    let peers = publish_peers(&replicated, 3).await;
    let visible = peers.first().expect("three peers");
    let (admin_server, admin, admin_root) = serve_membership(&[(visible, "Alive")]).await;
    let local = LocalMachine::new(store, watch::channel(false).0)
        .with_cluster(Some((replicated.clone(), AdminClient::new(&admin))));

    let registered = local
        .register(request("joiner", WireGuardPublicKey([1; 32])))
        .await
        .unwrap();
    assert_eq!(registered.assigned_machine.name.as_str(), "joiner");

    admin_server.abort();
    let _ = std::fs::remove_dir_all(admin_root);
    cluster.abort();
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}
