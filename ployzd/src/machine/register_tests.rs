use std::{
    collections::BTreeMap,
    convert::Infallible,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, body::Incoming, service::service_fn};
use hyper_util::rt::{TokioExecutor, TokioIo};
use ployz_core::{
    AdvertisedEndpoint, Machine, MachineName, MachineRuntime, MachineSubnet, RegisterRequest,
    WireGuardPublicKey,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::{LocalMachine, LocalMachineError, LocalMachineStore, StoreError};
use crate::corrosion::{AdminClient, ReplicatedStore};

#[derive(Deserialize)]
struct Statement {
    query: String,
    params: Vec<Value>,
}

struct ClusterKv {
    network: String,
    machines: BTreeMap<String, String>,
}

#[tokio::test]
async fn register_rejects_empty_endpoints_and_an_uninitialized_machine() {
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
    assert!(matches!(
        empty,
        LocalMachineError::Store(StoreError::MissingEndpoints)
    ));
    let uninitialized = local
        .register(request("peer", WireGuardPublicKey([1; 32])))
        .await
        .unwrap_err();
    assert!(matches!(uninitialized, LocalMachineError::NotParticipating));
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn register_assigns_a_free_subnet_publishes_and_rejects_duplicates() {
    let (local, replicated, founder, data_dir, server) = participating().await;
    let registered = local
        .register(request("peer", WireGuardPublicKey([1; 32])))
        .await
        .unwrap();
    assert_eq!(
        registered.assigned_machine.subnet,
        "10.210.1.0/24".parse().unwrap()
    );
    assert_eq!(registered.visible_peers, vec![founder.clone()]);
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
    assert!(matches!(
        duplicate_name,
        LocalMachineError::DuplicateMachine
    ));
    let duplicate_key = local
        .register(request("other", WireGuardPublicKey([1; 32])))
        .await
        .unwrap_err();
    assert!(matches!(duplicate_key, LocalMachineError::DuplicateMachine));

    server.abort();
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn overlapping_registers_on_one_daemon_get_distinct_machine_subnets() {
    let (local, replicated, _founder, data_dir, server) = participating().await;
    let publication = replicated.machine_publication().await;
    let first = {
        let local = local.clone();
        tokio::spawn(async move {
            local
                .register(request("peer-a", WireGuardPublicKey([1; 32])))
                .await
        })
    };
    let second = {
        let local = local.clone();
        tokio::spawn(async move {
            local
                .register(request("peer-b", WireGuardPublicKey([2; 32])))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !first.is_finished() && !second.is_finished(),
        "both Register calls must still be in flight"
    );
    drop(publication);
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();
    assert_ne!(
        first.assigned_machine.subnet,
        second.assigned_machine.subnet
    );
    let expected = [
        MachineSubnet::parse("10.210.1.0/24").unwrap(),
        MachineSubnet::parse("10.210.2.0/24").unwrap(),
    ];
    assert!(expected.contains(&first.assigned_machine.subnet));
    assert!(expected.contains(&second.assigned_machine.subnet));

    server.abort();
    drop(local);
    let _ = std::fs::remove_dir_all(data_dir);
}

async fn participating() -> (
    LocalMachine,
    ReplicatedStore,
    Machine,
    std::path::PathBuf,
    tokio::task::JoinHandle<()>,
) {
    let (address, server) = serve_cluster().await;
    let replicated = ReplicatedStore::connect(address, &"a".repeat(64));
    let data_dir = std::env::temp_dir().join(format!(
        "ployzd-register-{}",
        ployz_core::MachineId::random()
    ));
    let mut store = LocalMachineStore::open(&data_dir).unwrap();
    let founder = store
        .initialize(
            MachineName::parse("edge").unwrap(),
            "10.210.0.0/16".parse().unwrap(),
            None,
            vec![AdvertisedEndpoint("192.0.2.1:51820".parse().unwrap())],
            None,
        )
        .unwrap();
    replicated.publish_local_machine(&founder).await.unwrap();
    let local = LocalMachine::new(Arc::new(Mutex::new(store)), watch::channel(false).0)
        .with_cluster(Some((
            replicated.clone(),
            AdminClient::new("/no/such/ployz-admin.sock"),
        )));
    (local, replicated, founder, data_dir, server)
}

fn request(name: &str, public_key: WireGuardPublicKey) -> RegisterRequest {
    RegisterRequest {
        name: MachineName::parse(name).unwrap(),
        public_key,
        public_ip: None,
        advertised_endpoints: vec![AdvertisedEndpoint("192.0.2.9:51820".parse().unwrap())],
        runtime: MachineRuntime::default(),
    }
}

async fn serve_cluster() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let kv = Arc::new(Mutex::new(ClusterKv {
        network: "10.210.0.0/16".into(),
        machines: BTreeMap::new(),
    }));
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let kv = Arc::clone(&kv);
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let kv = Arc::clone(&kv);
                    async move { Ok::<_, Infallible>(dispatch(request, &kv).await) }
                });
                let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                    .serve_connection(io, service)
                    .await;
            });
        }
    });
    (address, server)
}

async fn dispatch(request: Request<Incoming>, kv: &Mutex<ClusterKv>) -> Response<Full<Bytes>> {
    let path = request.uri().path().to_owned();
    let body = request.into_body().collect().await.unwrap().to_bytes();
    let bytes = match path.as_str() {
        "/v1/queries" => query(kv, serde_json::from_slice(&body).unwrap()),
        "/v1/transactions" => execute(kv, serde_json::from_slice(&body).unwrap()),
        _ => {
            return Response::builder()
                .status(404)
                .body(Full::new(Bytes::new()))
                .unwrap();
        }
    };
    Response::new(Full::new(bytes))
}

fn query(kv: &Mutex<ClusterKv>, statement: Statement) -> Bytes {
    let kv = kv.lock().unwrap();
    if statement.query.contains("FROM machines") && statement.query.contains("ORDER BY") {
        events(
            &["id", "info"],
            kv.machines
                .iter()
                .map(|(id, info)| vec![json!(id), json!(info)]),
        )
    } else if statement.query.contains("FROM machines") {
        let id = text_param(&statement.params, 0);
        events(&["info"], kv.machines.get(id).map(|info| vec![json!(info)]))
    } else if statement.query.contains("FROM cluster") {
        events(&["value"], vec![vec![json!(kv.network)]])
    } else {
        events(&["site_id", "db_version"], Vec::new())
    }
}

fn execute(kv: &Mutex<ClusterKv>, statements: Vec<Statement>) -> Bytes {
    let mut kv = kv.lock().unwrap();
    for statement in &statements {
        if statement.query.contains("INSERT INTO machines") {
            kv.machines.insert(
                text_param(&statement.params, 0).to_owned(),
                text_param(&statement.params, 1).to_owned(),
            );
        }
    }
    serde_json::to_vec(&json!({
        "results": vec![json!({"rows_affected": 1, "time": 0.0}); statements.len()],
        "time": 0.0,
    }))
    .unwrap()
    .into()
}

fn text_param(params: &[Value], index: usize) -> &str {
    params
        .get(index)
        .and_then(Value::as_str)
        .expect("statement parameter")
}

fn events(columns: &[&str], rows: impl IntoIterator<Item = Vec<Value>>) -> Bytes {
    let mut body = serde_json::to_vec(&json!({ "columns": columns })).unwrap();
    for (index, row) in rows.into_iter().enumerate() {
        body.extend(serde_json::to_vec(&json!({ "row": [index as u64 + 1, row] })).unwrap());
    }
    body.extend(br#"{"eoq":{"time":0.0}}"#);
    body.into()
}
