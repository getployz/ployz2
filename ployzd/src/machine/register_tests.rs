use std::{
    net::Ipv6Addr,
    sync::{Arc, Mutex},
};

use axum::{Router, body::Bytes, extract::State, routing::post};
use ployz_core::{
    AdvertisedEndpoint, Machine, MachineId, MachineName, MachineRpc, MachineRpcServer,
    MachineRuntime, ManagementAddress, RegisterRequest, WireGuardPublicKey,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, transport::Server};

use super::{
    LocalMachine, LocalMachineError, LocalMachineStore, REGISTER_FORWARDED_METADATA, RegisterHop,
};
use crate::{
    corrosion::{AdminClient, ReplicatedStore},
    rpc::MachineService,
};

#[tokio::test]
async fn named_allocator_admits_register_locally() {
    let (dir, store, founder) = participating("ployzd-register-local");
    let replica = ClusterReplica::start(Some(founder.id), vec![founder.clone()]).await;
    let local =
        LocalMachine::new(store, tokio::sync::watch::channel(false).0).with_cluster(Some((
            replica.store.clone(),
            AdminClient::new("/no/such-admin.sock"),
        )));

    let registered = local.register(joiner_request()).await.unwrap();

    assert_eq!(registered.assigned_machine.name.as_str(), "joiner");
    assert_eq!(
        registered.assigned_machine.subnet.to_string(),
        "10.210.1.0/24"
    );
    assert!(
        replica
            .machines
            .lock()
            .unwrap()
            .iter()
            .any(|machine| machine.name.as_str() == "joiner")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn missing_allocator_row_does_not_admit() {
    let (dir, store, founder) = participating("ployzd-register-missing");
    let replica = ClusterReplica::start(None, vec![founder]).await;
    let local =
        LocalMachine::new(store, tokio::sync::watch::channel(false).0).with_cluster(Some((
            replica.store.clone(),
            AdminClient::new("/no/such-admin.sock"),
        )));

    let error = local.register(joiner_request()).await.unwrap_err();

    assert!(matches!(error, LocalMachineError::NotAllocator));
    assert!(
        !replica
            .machines
            .lock()
            .unwrap()
            .iter()
            .any(|machine| machine.name.as_str() == "joiner")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn contact_forwards_register_and_returns_the_allocator_payload() {
    let (allocator_dir, allocator_store, mut allocator_machine) =
        participating("ployzd-register-allocator");
    let listener = TcpListener::bind("[::1]:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    allocator_machine.management_address = ManagementAddress(Ipv6Addr::LOCALHOST);
    let allocator_replica =
        ClusterReplica::start(Some(allocator_machine.id), vec![allocator_machine.clone()]).await;
    let allocator = MachineService::with_cluster(
        allocator_store,
        tokio::sync::watch::channel(false).0,
        Some((
            allocator_replica.store.clone(),
            AdminClient::new("/no/such-admin.sock"),
        )),
    );
    tokio::spawn(
        Server::builder()
            .add_service(MachineRpcServer::new(allocator))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );

    let (contact_dir, contact_store, _contact) = participating("ployzd-register-contact");
    let contact_replica =
        ClusterReplica::start(Some(allocator_machine.id), vec![allocator_machine.clone()]).await;
    let contact = LocalMachine::new(contact_store, tokio::sync::watch::channel(false).0)
        .with_cluster(Some((
            contact_replica.store.clone(),
            AdminClient::new("/no/such-admin.sock"),
        )))
        .with_machine_api_port(port);

    let registered = contact.register(joiner_request()).await.unwrap();

    assert_eq!(registered.assigned_machine.name.as_str(), "joiner");
    assert_eq!(
        registered.assigned_machine.subnet.to_string(),
        "10.210.1.0/24"
    );
    assert!(
        allocator_replica
            .machines
            .lock()
            .unwrap()
            .iter()
            .any(|machine| machine.name.as_str() == "joiner"),
        "Allocator admits locally"
    );
    assert!(
        !contact_replica
            .machines
            .lock()
            .unwrap()
            .iter()
            .any(|machine| machine.name.as_str() == "joiner"),
        "contact must not allocate locally"
    );
    let _ = std::fs::remove_dir_all(allocator_dir);
    let _ = std::fs::remove_dir_all(contact_dir);
}

#[tokio::test]
async fn forwarded_register_does_not_admit_or_forward_when_kv_names_another_machine() {
    let (dir, store, local_machine) = participating("ployzd-register-one-hop");
    let other = MachineId::parse("b".repeat(32)).unwrap();
    let mut named = local_machine.clone();
    named.id = other;
    named.management_address = ManagementAddress(Ipv6Addr::LOCALHOST);
    let replica = ClusterReplica::start(Some(other), vec![named]).await;
    let local = LocalMachine::new(store, tokio::sync::watch::channel(false).0)
        .with_cluster(Some((
            replica.store.clone(),
            AdminClient::new("/no/such-admin.sock"),
        )))
        .with_machine_api_port(1);

    let error = local
        .register_at(joiner_request(), RegisterHop::Forwarded)
        .await
        .unwrap_err();

    assert!(matches!(error, LocalMachineError::NotAllocator));
    assert!(
        !replica
            .machines
            .lock()
            .unwrap()
            .iter()
            .any(|machine| machine.name.as_str() == "joiner")
    );
    assert!(
        !replica
            .executes
            .lock()
            .unwrap()
            .iter()
            .any(|query| query.contains("allocator")),
        "no steal"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn unreachable_allocator_does_not_steal() {
    let (dir, store, _local_machine) = participating("ployzd-register-unreachable");
    let allocator_id = MachineId::parse("c".repeat(32)).unwrap();
    let named = Machine {
        id: allocator_id,
        name: MachineName::parse("allocator").unwrap(),
        subnet: "10.210.0.0/24".parse().unwrap(),
        management_address: ManagementAddress(Ipv6Addr::LOCALHOST),
        public_key: WireGuardPublicKey([3; 32]),
        public_ip: None,
        advertised_endpoints: vec![AdvertisedEndpoint("192.0.2.3:51820".parse().unwrap())],
        runtime: MachineRuntime::default(),
    };
    let replica = ClusterReplica::start(Some(allocator_id), vec![named]).await;
    let local = LocalMachine::new(store, tokio::sync::watch::channel(false).0)
        .with_cluster(Some((
            replica.store.clone(),
            AdminClient::new("/no/such-admin.sock"),
        )))
        .with_machine_api_port(1);

    let error = local.register(joiner_request()).await.unwrap_err();

    assert!(matches!(error, LocalMachineError::AllocatorUnreachable));
    assert!(
        !replica
            .executes
            .lock()
            .unwrap()
            .iter()
            .any(|query| query.contains("allocator")),
        "no steal"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn forwarded_rpc_metadata_admits_locally_only() {
    let (dir, store, founder) = participating("ployzd-register-metadata");
    let replica = ClusterReplica::start(Some(founder.id), vec![founder.clone()]).await;
    let service = MachineService::with_cluster(
        store,
        tokio::sync::watch::channel(false).0,
        Some((
            replica.store.clone(),
            AdminClient::new("/no/such-admin.sock"),
        )),
    );
    let mut request = Request::new(
        ployz_core::op::Register::into_request(joiner_request())
            .encode()
            .unwrap(),
    );
    request.metadata_mut().insert(
        REGISTER_FORWARDED_METADATA,
        "1".parse().expect("ASCII metadata"),
    );

    let registered = service
        .register(request)
        .await
        .unwrap()
        .into_inner()
        .decode_response()
        .unwrap()
        .decode::<ployz_core::op::Register>()
        .unwrap();

    assert_eq!(registered.assigned_machine.name.as_str(), "joiner");
    let _ = std::fs::remove_dir_all(dir);
}

fn participating(prefix: &str) -> (std::path::PathBuf, Arc<Mutex<LocalMachineStore>>, Machine) {
    let dir = std::env::temp_dir().join(format!("{prefix}-{}", MachineId::random()));
    let mut store = LocalMachineStore::open(&dir).unwrap();
    let machine = store
        .initialize(
            MachineName::parse("edge").unwrap(),
            "10.210.0.0/16".parse().unwrap(),
            None,
            vec![AdvertisedEndpoint("192.0.2.1:51820".parse().unwrap())],
            None,
        )
        .unwrap();
    (dir, Arc::new(Mutex::new(store)), machine)
}

fn joiner_request() -> RegisterRequest {
    RegisterRequest {
        name: MachineName::parse("joiner").unwrap(),
        public_key: WireGuardPublicKey([7; 32]),
        public_ip: None,
        advertised_endpoints: vec![AdvertisedEndpoint("192.0.2.9:51820".parse().unwrap())],
        runtime: MachineRuntime::default(),
    }
}

#[derive(Clone)]
struct ClusterReplica {
    store: ReplicatedStore,
    allocator: Arc<Mutex<Option<MachineId>>>,
    machines: Arc<Mutex<Vec<Machine>>>,
    executes: Arc<Mutex<Vec<String>>>,
}

impl ClusterReplica {
    async fn start(allocator: Option<MachineId>, machines: Vec<Machine>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let replica = Self {
            store: ReplicatedStore::http1(address),
            allocator: Arc::new(Mutex::new(allocator)),
            machines: Arc::new(Mutex::new(machines)),
            executes: Arc::new(Mutex::new(Vec::new())),
        };
        let serving = replica.clone();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/v1/queries", post(cluster_query))
                    .route("/v1/transactions", post(cluster_execute))
                    .with_state(serving),
            )
            .await
            .unwrap();
        });
        replica
    }
}

#[derive(Deserialize)]
struct ClusterStatement {
    query: String,
    #[serde(default)]
    params: Vec<Value>,
}

async fn cluster_query(State(replica): State<ClusterReplica>, body: Bytes) -> Vec<u8> {
    let statement: ClusterStatement = serde_json::from_slice(&body).unwrap();
    let query = statement.query.as_str();
    if query.contains("crsql_db_versions") {
        return query_body(&["site_id", "db_version"], Vec::new());
    }
    if query.contains("key = 'allocator'") {
        let allocator = replica.allocator.lock().unwrap();
        return match &*allocator {
            Some(id) => query_body(&["value"], vec![vec![json!(id.as_str())]]),
            None => query_body(&["value"], Vec::new()),
        };
    }
    if query.contains("key = 'network'") {
        return query_body(&["value"], vec![vec![json!("10.210.0.0/16")]]);
    }
    if query.contains("FROM machines WHERE id") {
        let id = statement.params.first().and_then(Value::as_str).unwrap();
        let machines = replica.machines.lock().unwrap();
        return match machines.iter().find(|machine| machine.id.as_str() == id) {
            Some(machine) => query_body(
                &["info"],
                vec![vec![json!(serde_json::to_string(machine).unwrap())]],
            ),
            None => query_body(&["info"], Vec::new()),
        };
    }
    if query.contains("FROM machines") {
        let machines = replica.machines.lock().unwrap();
        let rows = machines
            .iter()
            .map(|machine| {
                vec![
                    json!(machine.id.as_str()),
                    json!(serde_json::to_string(machine).unwrap()),
                ]
            })
            .collect();
        return query_body(&["id", "info"], rows);
    }
    query_body(&["value"], Vec::new())
}

async fn cluster_execute(State(replica): State<ClusterReplica>, body: Bytes) -> String {
    let statements: Vec<ClusterStatement> = serde_json::from_slice(&body).unwrap();
    for statement in statements {
        replica
            .executes
            .lock()
            .unwrap()
            .push(statement.query.clone());
        if statement.query.contains("INSERT INTO machines") {
            let info = statement.params.get(1).and_then(Value::as_str).unwrap();
            let machine: Machine = serde_json::from_str(info).unwrap();
            replica.machines.lock().unwrap().push(machine);
        }
    }
    serde_json::to_string(&json!({
        "results": [{"rows_affected": 1, "time": 0.0}],
        "time": 0.0
    }))
    .unwrap()
}

fn query_body(columns: &[&str], rows: Vec<Vec<Value>>) -> Vec<u8> {
    let mut body = serde_json::to_vec(&json!({ "columns": columns })).unwrap();
    for (index, row) in rows.into_iter().enumerate() {
        body.extend(serde_json::to_vec(&json!({ "row": [index, row] })).unwrap());
    }
    body.extend(serde_json::to_vec(&json!({ "eoq": { "time": 0.0 } })).unwrap());
    body
}
