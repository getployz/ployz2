//! Caddy watcher recovery at the Corrosion HTTP boundary.

use super::{ingress, observation};
use crate::{
    caddy::{CONFIG_FILE, run},
    corrosion::{ApiClient, ReplicatedStore},
};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
    response::Response,
    routing::post,
};
use futures_util::StreamExt;
use ployz_core::{
    AdvertisedEndpoint, ContainerObservation, HttpProtocol, Machine, MachineId, MachineName,
    ManagementAddress, WireGuardPublicKey,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, sync::mpsc};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn retries_protocol_failures_without_exiting() {
    let machine = Machine {
        id: MachineId::parse("a".repeat(32)).unwrap(),
        name: MachineName::parse("node-a").unwrap(),
        subnet: "10.210.1.0/24".parse().unwrap(),
        management_address: ManagementAddress("fdcc::1".parse().unwrap()),
        public_key: WireGuardPublicKey([1; 32]),
        public_ip: None,
        advertised_endpoints: vec![AdvertisedEndpoint("192.0.2.1:51000".parse().unwrap())],
        runtime: Default::default(),
    };
    let state = WatchState {
        container: observation(
            1,
            &machine.id,
            "api",
            Some([10, 210, 1, 2]),
            vec![ingress("example.com", 80, HttpProtocol::Http)],
        ),
        container_subscriptions: Arc::new(AtomicUsize::new(0)),
        holds: Arc::new(Mutex::new(Vec::new())),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/v1/queries", post(query))
                .route("/v1/subscriptions", post(subscribe))
                .with_state(server_state),
        )
        .await
        .unwrap();
    });
    let directory = std::env::temp_dir().join(format!(
        "ployz-caddy-watch-retry-test-{}",
        MachineId::random()
    ));
    let caddyfile = directory.join(CONFIG_FILE);
    let shutdown = CancellationToken::new();
    let watcher = tokio::spawn(run(
        machine,
        ReplicatedStore::new(ApiClient::http1(address, &"a".repeat(64)).unwrap()),
        caddyfile.clone(),
        directory.join("missing-admin.sock"),
        shutdown.clone(),
    ));

    wait_for_caddyfile(&caddyfile, "10.210.1.2:80").await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while state.container_subscriptions.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Caddy watcher did not resubscribe");
    assert!(!watcher.is_finished(), "watcher failure exited the plane");

    shutdown.cancel();
    watcher.await.unwrap().unwrap();
    server.abort();
    std::fs::remove_dir_all(directory).unwrap();
}

#[derive(Clone)]
struct WatchState {
    container: ContainerObservation,
    container_subscriptions: Arc<AtomicUsize>,
    holds: Arc<Mutex<Vec<mpsc::UnboundedSender<Bytes>>>>,
}

#[derive(Deserialize)]
struct Statement {
    query: String,
    #[serde(rename = "params")]
    _params: Vec<Value>,
}

async fn query(State(state): State<WatchState>, body: Bytes) -> Bytes {
    let statement: Statement = serde_json::from_slice(&body).unwrap();
    if statement.query == "SELECT id, container FROM containers ORDER BY id" {
        query_events(
            &["id", "container"],
            [vec![
                json!(state.container.container_id),
                json!(serde_json::to_string(&state.container).unwrap()),
            ]],
        )
    } else if statement.query == "SELECT hostname, body FROM certificates ORDER BY hostname" {
        query_events(&["hostname", "body"], [])
    } else {
        panic!("unexpected query {}", statement.query);
    }
}

async fn subscribe(State(state): State<WatchState>, body: Bytes) -> Response {
    let statement: Statement = serde_json::from_slice(&body).unwrap();
    let (columns, fail) = if statement.query == "SELECT id, container FROM containers" {
        (
            &["id", "container"][..],
            state.container_subscriptions.fetch_add(1, Ordering::SeqCst) == 0,
        )
    } else if statement.query == "SELECT hostname, body FROM certificates" {
        (&["hostname", "body"][..], false)
    } else {
        panic!("unexpected subscription {}", statement.query);
    };
    let (sender, receiver) = mpsc::unbounded_channel();
    sender
        .send(Bytes::from(format!(
            "{}\n{{\"eoq\":{{\"time\":0.0}}}}\n",
            json!({ "columns": columns })
        )))
        .unwrap();
    if fail {
        sender
            .send(Bytes::from_static(b"{\"row\":[1,[]]}\n"))
            .unwrap();
    }
    state.holds.lock().unwrap().push(sender);
    Response::new(Body::from_stream(
        UnboundedReceiverStream::new(receiver).map(Ok::<_, Infallible>),
    ))
}

fn query_events(columns: &[&str], rows: impl IntoIterator<Item = Vec<Value>>) -> Bytes {
    let mut body = serde_json::to_vec(&json!({ "columns": columns })).unwrap();
    for (index, row) in rows.into_iter().enumerate() {
        body.extend(serde_json::to_vec(&json!({ "row": [index as u64 + 1, row] })).unwrap());
    }
    body.extend(br#"{"eoq":{"time":0.0}}"#);
    body.into()
}

async fn wait_for_caddyfile(path: &Path, expected: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if std::fs::read_to_string(path).is_ok_and(|caddyfile| caddyfile.contains(expected)) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("Caddyfile never contained {expected}"));
}
