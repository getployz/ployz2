use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use axum::{Router, body::Bytes, extract::State, routing::post};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::store::{ALLOCATOR_ROW, CLAIM_FOUNDER_ALLOCATOR};
use super::{ApiClient, ReplicatedStore};
use ployz_core::MachineId;

#[derive(Clone)]
struct ClusterKv {
    network: String,
    machines: BTreeMap<String, String>,
    allocator: Option<(String, bool)>,
}

#[derive(Deserialize)]
struct Statement {
    query: String,
    params: Vec<Value>,
}

pub(crate) async fn store() -> (ReplicatedStore, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let kv = Arc::new(Mutex::new(ClusterKv {
        network: "10.210.0.0/16".into(),
        machines: BTreeMap::new(),
        allocator: None,
    }));
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/v1/queries", post(queries))
                .route("/v1/transactions", post(transactions))
                .with_state(kv),
        )
        .await
        .unwrap();
    });
    (
        ReplicatedStore::new(ApiClient::http1(address, &"a".repeat(64)).unwrap()),
        server,
    )
}

async fn queries(State(kv): State<Arc<Mutex<ClusterKv>>>, body: Bytes) -> Vec<u8> {
    query(&kv, serde_json::from_slice(&body).unwrap()).into()
}

async fn transactions(State(kv): State<Arc<Mutex<ClusterKv>>>, body: Bytes) -> Vec<u8> {
    execute(&kv, serde_json::from_slice(&body).unwrap()).into()
}

fn query(kv: &Mutex<ClusterKv>, statement: Statement) -> Bytes {
    let kv = kv.lock().unwrap();
    match statement.query.as_str() {
        "SELECT id, info FROM machines ORDER BY name" => events(
            &["id", "info"],
            kv.machines
                .iter()
                .map(|(id, info)| vec![json!(id), json!(info)]),
        ),
        "SELECT info FROM machines WHERE id = ?" => {
            let id = text_param(&statement.params, 0);
            events(&["info"], kv.machines.get(id).map(|info| vec![json!(info)]))
        }
        "SELECT value FROM cluster WHERE key = 'network'" => {
            events(&["value"], vec![vec![json!(kv.network)]])
        }
        ALLOCATOR_ROW => events(
            &["allocator", "quiet"],
            kv.allocator
                .iter()
                .map(|(id, quiet)| vec![json!(id), json!(u8::from(*quiet))]),
        ),
        "SELECT site_id, db_version FROM crsql_db_versions" => {
            events(&["site_id", "db_version"], Vec::new())
        }
        query => panic!("unexpected query {query}"),
    }
}

fn execute(kv: &Mutex<ClusterKv>, statements: Vec<Statement>) -> Bytes {
    let mut kv = kv.lock().unwrap();
    for statement in &statements {
        match statement.query.as_str() {
            CLAIM_FOUNDER_ALLOCATOR => {
                let id = text_param(&statement.params, 0).to_owned();
                kv.allocator.get_or_insert((id, true));
            }
            query if query.starts_with("INSERT INTO machines (id, info,") => {
                kv.machines.insert(
                    text_param(&statement.params, 0).to_owned(),
                    text_param(&statement.params, 1).to_owned(),
                );
            }
            query if query.contains("VALUES ('allocator'") => {
                kv.allocator = Some((
                    text_param(&statement.params, 0).to_owned(),
                    query.contains("-5 seconds"),
                ));
            }
            query => panic!("unexpected statement {query}"),
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

pub(crate) async fn set_allocator(store: &ReplicatedStore, id: &MachineId, quiet: bool) {
    let updated_at = if quiet {
        "datetime('now', '-5 seconds')"
    } else {
        "datetime('now')"
    };
    store
        .api()
        .execute([super::Statement::new(
            format!(
                "INSERT INTO cluster (key, value, updated_at) VALUES ('allocator', ?, {updated_at}) ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at"
            ),
            [json!(id)],
        )])
        .await
        .unwrap();
}
