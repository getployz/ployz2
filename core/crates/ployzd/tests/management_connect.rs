//! Connection contract of the management transport: the public client connector against
//! a real Machine API served over an in-process relay, asserting only what a client sees.

use std::{
    collections::HashSet,
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr, RelayMode, SecretKey,
    endpoint::{WeakConnectionHandle, presets},
    test_utils::run_relay_server,
    tls::CaTlsConfig,
};
use ployz::{
    connect::{ConnectError, Connector, ManagementRelay, SystemConnector},
    context::Connection,
};
use ployz_core::{
    AdvertisedEndpoint, CloudPairing, DescribeContractRequest, InitializeRequest, MANAGEMENT_ALPN,
    MachineId, MachineName, MachineRpcClient, ManagementCapability, PairingCredential,
    RpcErrorCode, SetCloudPairingRequest, op,
};
use ployzd::{
    machine::{LocalMachine, LocalMachineStore, RecordOwner},
    machine_api::MachineApi,
    management::{self, ManagementConfig},
};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;
use tonic::{
    body::Body,
    codegen::{Service, http},
    transport::Channel,
};

#[tokio::test(flavor = "multi_thread")]
async fn client_connector_honours_the_management_transport_contract() {
    tokio::time::timeout(Duration::from_secs(120), contract())
        .await
        .expect("contract test timed out");
}

async fn contract() {
    let (_relay_map, relay_url, _relay) = run_relay_server().await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let owner = RecordOwner::spawn(LocalMachineStore::open(dir.path()).unwrap()).unwrap();
    let local = LocalMachine::new(owner.clone());
    local
        .initialize(InitializeRequest {
            initial_policy: Default::default(),
            name: MachineName::parse("first").unwrap(),
            cluster_network: "10.210.0.0/16".parse().unwrap(),
            public_ip: None,
            advertised_endpoints: vec![AdvertisedEndpoint("192.0.2.1:51820".parse().unwrap())],
            wireguard_mtu: None,
            cloud_pairing: None,
        })
        .await
        .unwrap();
    let machine_id = local.record().id();
    let capability = local
        .set_cloud_pairing(SetCloudPairingRequest::Set {
            pairing: CloudPairing::new(PairingCredential::parse("pairing").unwrap()),
        })
        .await
        .unwrap()
        .capability
        .unwrap();

    let config = ManagementConfig {
        relay_url: relay_url.clone(),
        port: 0,
        relay_tls: CaTlsConfig::insecure_skip_verify(),
    };
    let endpoint = management::bind(local.record().management_secret(), &config)
        .await
        .unwrap();
    let shutdown = CancellationToken::new();
    let (connections, mut observed) = mpsc::unbounded_channel();
    let gate = Arc::new(RequestGate::default());
    let api = ObservedApi {
        api: MachineApi::builder(owner.clone()).build(),
        connections,
        seen: Arc::default(),
        gate: Arc::clone(&gate),
    };
    let server = tokio::spawn(management::serve(
        endpoint.clone(),
        owner.watch(),
        api,
        shutdown.clone(),
    ));
    let connector = Arc::new(
        SystemConnector::new("/ployz-missing-ssh-client").with_management_relay(
            ManagementRelay::custom(relay_url.clone(), CaTlsConfig::insecure_skip_verify()),
        ),
    );

    // A key the Machine never accepted is refused by identity, not by reachability.
    let stranger =
        ManagementCapability::new(*capability.machine(), SecretKey::generate().to_bytes());
    assert_refused(connector.connect(&connection(&stranger)).await);

    // Fifty concurrent RPC streams over one accepted connection all complete.
    let channel = connector.connect(&connection(&capability)).await.unwrap();
    let surviving_connection = observed.recv().await.unwrap();
    gate.hold.store(true, Ordering::SeqCst);
    let calls: Vec<_> = (0..50)
        .map(|_| tokio::spawn(describe(channel.clone())))
        .collect();
    wait_until(|| gate.active.load(Ordering::SeqCst) == 50).await;
    gate.hold.store(false, Ordering::SeqCst);
    gate.release.add_permits(50);
    for call in calls {
        assert_eq!(call.await.unwrap().unwrap(), machine_id);
    }

    // Cancelling in-flight streams and dropping their connection leaves the surviving
    // connection and fresh dials unaffected.
    let cancelled = connector.connect(&connection(&capability)).await.unwrap();
    let cancelled_connection = observed.recv().await.unwrap();
    gate.hold.store(true, Ordering::SeqCst);
    let in_flight: Vec<_> = (0..50)
        .map(|_| tokio::spawn(describe(cancelled.clone())))
        .collect();
    wait_until(|| gate.active.load(Ordering::SeqCst) == 50).await;
    for call in &in_flight {
        call.abort();
    }
    for call in in_flight {
        assert!(call.await.unwrap_err().is_cancelled());
    }
    drop(cancelled);
    tokio::time::timeout(Duration::from_secs(10), cancelled_connection.closed())
        .await
        .unwrap();
    wait_until(|| gate.active.load(Ordering::SeqCst) == 0).await;
    gate.hold.store(false, Ordering::SeqCst);
    assert_eq!(describe(channel.clone()).await.unwrap(), machine_id);
    let session = ployz::sdk::connect_connections(vec![connection(&capability)], connector.clone())
        .await
        .unwrap();
    let removal_connection = observed.recv().await.unwrap();

    // Revocation must also reach a peer that completed TLS but delayed its first stream.
    // Use a distinct identity to avoid replacing the connector's relay registration.
    let delayed_capability = local
        .set_cloud_pairing(SetCloudPairingRequest::Set {
            pairing: CloudPairing::new(PairingCredential::parse("delayed").unwrap()),
        })
        .await
        .unwrap()
        .capability
        .unwrap();
    let delayed_secret = SecretKey::from_bytes(delayed_capability.client_secret());
    let delayed = Endpoint::builder(presets::Minimal)
        .secret_key(delayed_secret)
        .relay_mode(RelayMode::custom([relay_url.clone()]))
        .ca_tls_config(CaTlsConfig::insecure_skip_verify())
        .bind()
        .await
        .unwrap();
    let waiting = delayed
        .connect(
            EndpointAddr::new(endpoint.id()).with_relay_url(relay_url),
            MANAGEMENT_ALPN,
        )
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), waiting.closed())
            .await
            .is_err(),
        "accepted peer must remain connected before rotation"
    );
    let capability = local
        .set_cloud_pairing(SetCloudPairingRequest::Set {
            pairing: CloudPairing::new(PairingCredential::parse("next").unwrap()),
        })
        .await
        .unwrap()
        .capability
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), waiting.closed())
        .await
        .expect("rotation must revoke a peer waiting to open its first stream");
    delayed.close().await;
    drop(session);
    tokio::time::timeout(Duration::from_secs(10), removal_connection.closed())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), surviving_connection.closed())
        .await
        .unwrap();
    drop(channel);

    // Exercise Cloud's actual SDK path. A lost Clear response is not confirmation;
    // the subsequent SDK connect must preserve the daemon's identity refusal.
    let session = ployz::sdk::connect_connections(vec![connection(&capability)], connector.clone())
        .await
        .unwrap();
    let removal_connection = observed.recv().await.unwrap();
    let _ = session.remove_cloud_pairing().await;
    tokio::time::timeout(Duration::from_secs(10), removal_connection.closed())
        .await
        .unwrap();
    drop(session);
    let error =
        match ployz::sdk::connect_connections(vec![connection(&capability)], connector.clone())
            .await
        {
            Ok(_) => panic!("removed capability must not reconnect through the SDK"),
            Err(error) => error,
        };
    assert_eq!(error.code, RpcErrorCode::Unauthenticated);
    assert_refused(connector.connect(&connection(&capability)).await);

    // Shutdown drains every remaining connection and closes the endpoint.
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("serve must finish once every client is gone")
        .unwrap()
        .unwrap();
    assert!(endpoint.is_closed());
}

fn assert_refused(result: Result<Channel, ConnectError>) {
    match result {
        Err(ConnectError::RefusedByIdentity) => {}
        Err(error) => panic!("expected refusal by identity, got: {error} ({error:?})"),
        Ok(_) => panic!("expected refusal by identity, got a channel"),
    }
}

fn connection(capability: &ManagementCapability) -> Connection {
    Connection::management(capability.to_secret_string()).unwrap()
}

async fn describe(channel: Channel) -> Result<MachineId, tonic::Status> {
    let response = MachineRpcClient::new(channel)
        .describe_contract(
            op::DescribeContract::into_request(DescribeContractRequest {})
                .encode()
                .unwrap(),
        )
        .await?;
    Ok(response
        .into_inner()
        .decode_response()
        .unwrap()
        .decode::<op::DescribeContract>()
        .unwrap()
        .machine_id)
}

async fn wait_until(predicate: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("observable connection state did not settle");
}

struct RequestGate {
    hold: AtomicBool,
    active: Arc<AtomicUsize>,
    release: Semaphore,
}

impl Default for RequestGate {
    fn default() -> Self {
        Self {
            hold: AtomicBool::new(false),
            active: Arc::default(),
            release: Semaphore::new(0),
        }
    }
}

struct ActiveRequest(Arc<AtomicUsize>);
impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

// Observe the connection metadata supplied to the real Machine API and hold requests
// at the service boundary so concurrency and cancellation cannot pass by scheduling luck.
#[derive(Clone)]
struct ObservedApi {
    api: MachineApi,
    connections: mpsc::UnboundedSender<WeakConnectionHandle>,
    seen: Arc<Mutex<HashSet<usize>>>,
    gate: Arc<RequestGate>,
}

impl Service<http::Request<Body>> for ObservedApi {
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.api.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        let weak = request
            .extensions()
            .get::<WeakConnectionHandle>()
            .unwrap()
            .clone();
        let connection = weak.upgrade().unwrap();
        if self.seen.lock().unwrap().insert(connection.stable_id()) {
            self.connections.send(weak).unwrap();
        }
        let future = self.api.call(request);
        let gate = Arc::clone(&self.gate);
        Box::pin(async move {
            if gate.hold.load(Ordering::SeqCst) {
                gate.active.fetch_add(1, Ordering::SeqCst);
                let _active = ActiveRequest(Arc::clone(&gate.active));
                gate.release.acquire().await.unwrap().forget();
            }
            future.await
        })
    }
}
