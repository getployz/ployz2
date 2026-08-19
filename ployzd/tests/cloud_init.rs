mod test_dir;

use std::{
    fs,
    sync::{Arc, Mutex},
    time::Duration,
};

use ployz_core::{LocalMachinePhase, MachineId, MachineRpcServer};
use ployz_relay::{
    AUTHORIZATION_METADATA, CloudRelayClient, DialCredential, ListRequest, PAIRING_METADATA, Relay,
};
use ployzd::{machine::LocalMachineStore, relay::hold_register, rpc::MachineService};
use test_dir::TestDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UnixListener},
    sync::watch,
};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::{Request, metadata::MetadataValue, transport::Server};

const TOKEN: &str = "pmet_init";
const PAIRING: &str = "pairing-secret";
const DIAL: &str = "dial-secret";

#[tokio::test]
async fn cloud_init_initialize_participates_with_pairing_and_list_held() {
    let root = TestDir::new("ployz-cloud-init");
    fs::create_dir_all(&root.0).unwrap();
    let data = root.0.join("data");
    let run = root.0.join("run");
    fs::create_dir_all(&run).unwrap();
    let socket = run.join("ployz.sock");
    let config = root.0.join("config.yaml");

    let relay = Relay::new(DialCredential::parse(DIAL).unwrap());
    let (relay_addr, _relay_server, _goaway) = relay
        .serve((std::net::Ipv4Addr::LOCALHOST, 0).into())
        .await
        .unwrap();
    let relay_url = format!("http://{relay_addr}");
    let enroll = serve_initialize_enroll(&relay_url, PAIRING).await;

    let store = Arc::new(Mutex::new(LocalMachineStore::open(&data).unwrap()));
    let machine_id = store.lock().unwrap().record().id();
    let (reset, _) = watch::channel(false);
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(
        Server::builder()
            .add_service(MachineRpcServer::new(MachineService::new(
                Arc::clone(&store),
                reset,
            )))
            .serve_with_incoming(UnixListenerStream::new(listener)),
    );

    let connect = format!("unix://{}", socket.display());
    let enroll_url = enroll.url.clone();
    let config_path = config.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut command = ployz::cli::command();
        let matches = command
            .clone()
            .try_get_matches_from([
                "ployz",
                "--ployz-config",
                config_path.to_str().unwrap(),
                "--connect",
                &connect,
                "init",
                "--cloud",
                TOKEN,
                "--cloud-url",
                &enroll_url,
                "--name",
                "founder",
                "--network",
                "10.210.0.0/16",
                "--wg-mtu",
                "1400",
                "--no-caddy",
                "--no-dns",
                "--yes",
            ])
            .unwrap();
        ployz::handlers::dispatch(&matches, &mut command)
    })
    .await
    .unwrap();
    result.unwrap();

    let posted = enroll.posted.await.unwrap();
    assert_eq!(posted, format!("/api/enroll/{TOKEN}"));

    let record = store.lock().unwrap().record().clone();
    assert_eq!(record.phase(), LocalMachinePhase::Participating);
    assert_eq!(record.id(), machine_id);
    assert_eq!(
        record.cluster_network().unwrap().to_string(),
        "10.210.0.0/16"
    );
    assert_eq!(record.wireguard_mtu, Some(1400));
    let pairing = record.cloud_pairing.expect("Cloud Pairing is stored");
    assert_eq!(pairing.relay_url(), relay_url);
    assert_eq!(pairing.secret().as_str(), PAIRING);
    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(data.join("machine.json")).unwrap()).unwrap();
    assert!(persisted.get("dial").is_none());
    assert!(
        persisted
            .get("cloud_pairing")
            .and_then(|value| value.get("dial"))
            .is_none()
    );

    let _hold = hold_register(pairing.relay_url(), pairing.secret(), &machine_id)
        .await
        .unwrap();
    wait_for_held(&relay_url, machine_id).await;

    server.abort();
}

struct EnrollStub {
    url: String,
    posted: tokio::task::JoinHandle<String>,
}

async fn serve_initialize_enroll(relay_url: &str, secret: &str) -> EnrollStub {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let payload = serde_json::json!({
        "kind": "initialize",
        "pairing": {
            "relayUrl": relay_url,
            "secret": secret,
        },
    })
    .to_string();
    let posted = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0; 16384];
        let n = stream.read(&mut buf).await.unwrap();
        let request =
            String::from_utf8_lossy(buf.get(..n).expect("read count fits buffer")).into_owned();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            payload.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        request
            .lines()
            .next()
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .to_owned()
    });
    EnrollStub {
        url: format!("http://{addr}"),
        posted,
    }
}

async fn wait_for_held(url: &str, machine_id: MachineId) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let listed = list_held(url).await;
        if listed.iter().any(|id| id == &machine_id) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected {machine_id} on listHeld, got {listed:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn list_held(url: &str) -> Vec<MachineId> {
    let mut client = CloudRelayClient::new(
        tonic::transport::Endpoint::from_shared(url.to_owned())
            .unwrap()
            .connect()
            .await
            .unwrap(),
    );
    let mut request = Request::new(ListRequest {});
    request.metadata_mut().insert(
        AUTHORIZATION_METADATA,
        format!("Bearer {DIAL}").parse().unwrap(),
    );
    request
        .metadata_mut()
        .insert(PAIRING_METADATA, MetadataValue::try_from(PAIRING).unwrap());
    client
        .list(request)
        .await
        .unwrap()
        .into_inner()
        .registers()
        .iter()
        .map(|row| row.machine_id().unwrap())
        .collect()
}
