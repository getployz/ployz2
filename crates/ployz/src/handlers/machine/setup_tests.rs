//! Real setup RPC deadlines and initialization recovery under virtual time.

use std::{
    convert::Infallible,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures_util::StreamExt;
use ployz_core::{
    InitializeRequest, InspectRequest, LocalMachinePhase, Machine, MachineDetails, MachineId,
    MachineName, OpaquePayload, RpcRequestBody, RpcResponse, WireGuardPublicKey, op,
};
use tokio::time::Instant;
use tonic::{
    Request, Response, Status,
    codec::ProstCodec,
    transport::{Channel, Server},
};

use crate::{
    connect::{BoxProxyStream, Client, ConnectError, Connector},
    context::{Connection, ConnectionSource},
};

struct Connected(Channel);

#[tonic::async_trait]
impl Connector for Connected {
    async fn connect(&self, _: &Connection) -> Result<Channel, ConnectError> {
        Ok(self.0.clone())
    }
    async fn dial_proxy(
        &self,
        _: &Connection,
        _: &str,
        _: &str,
    ) -> Result<BoxProxyStream, ConnectError> {
        unreachable!("setup RPCs do not open a proxy")
    }
}

async fn starting_machine() -> (
    Client,
    Machine,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
) {
    let machine = Machine {
        id: MachineId::random(),
        name: MachineName::parse("founder").unwrap(),
        subnet: "10.210.0.0/24".parse().unwrap(),
        public_key: WireGuardPublicKey([2; 32]),
        public_ip: None,
        advertised_endpoints: Vec::new(),
        runtime: Default::default(),
    };
    let initialized = Arc::new(AtomicUsize::new(0));
    let calls = initialized.clone();
    let observed = machine.clone();
    let ready = Instant::now() + Duration::from_secs(70);
    let rpc = tower::service_fn(move |request: Request<OpaquePayload>| {
        let calls = calls.clone();
        let machine = observed.clone();
        async move {
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "this fixture rejects every RPC except Initialize and Inspect"
            )]
            match request.into_inner().decode_request().unwrap().body {
                RpcRequestBody::Initialize(_) => {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::future::pending::<Result<Response<OpaquePayload>, Status>>().await
                }
                RpcRequestBody::Inspect(_) => {
                    let started = calls.load(Ordering::SeqCst) > 0;
                    if started {
                        tokio::time::sleep_until(ready).await;
                    }
                    let details = MachineDetails {
                        id: machine.id,
                        phase: if started {
                            LocalMachinePhase::Participating
                        } else {
                            LocalMachinePhase::Uninitialized
                        },
                        public_key: machine.public_key,
                        advertised_endpoints: Vec::new(),
                        machine: started.then_some(machine),
                        store_version: Default::default(),
                        rtts: Vec::new(),
                        cloud_paired: false,
                        telemetry: None,
                        storage: None,
                    };
                    Ok(Response::new(RpcResponse::from(details).encode().unwrap()))
                }
                request => panic!("unexpected setup request: {request:?}"),
            }
        }
    });
    let service = tower::service_fn(move |request: http::Request<tonic::body::Body>| {
        let rpc = rpc.clone();
        async move {
            Ok::<_, Infallible>(
                tonic::server::Grpc::new(ProstCodec::default())
                    .unary(rpc, request)
                    .await,
            )
        }
    });
    // Both peers run inside Tokio; clock advancement cannot outrun socket I/O.
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let incoming =
        tokio_stream::once(Ok::<_, std::io::Error>(server_io)).chain(tokio_stream::pending());
    let server = tokio::spawn(Server::builder().serve_with_incoming(service, incoming));
    let client_io = Arc::new(Mutex::new(Some(client_io)));
    let channel = Channel::from_static("http://memory.invalid")
        .connect_with_connector(tower::service_fn(move |_| {
            std::future::ready(Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(
                client_io.lock().unwrap().take().unwrap(),
            )))
        }))
        .await
        .unwrap();
    let client = Client::new(
        channel.clone(),
        Connection::tcp("127.0.0.1:1".parse().unwrap()),
        ConnectionSource::Direct,
        Arc::new(Connected(channel)),
    );
    (client, machine, initialized, server)
}

fn initialize_request(machine: &Machine) -> InitializeRequest {
    InitializeRequest {
        name: machine.name.clone(),
        cluster_network: "10.210.0.0/16".parse().unwrap(),
        public_ip: None,
        advertised_endpoints: Vec::new(),
        wireguard_mtu: None,
        cloud_pairing: None,
    }
}

#[tokio::test(start_paused = true)]
async fn initialize_recovers_beyond_the_read_budget_without_replaying_the_mutation() {
    let started = Instant::now();
    let (mut client, machine, calls, server) = starting_machine().await;
    let result = super::initialize(&mut client, initialize_request(&machine))
        .await
        .unwrap();
    assert_eq!(result.machine, machine);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(Instant::now() - started, Duration::from_secs(70));
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn held_setup_rpcs_keep_the_mutation_and_read_deadlines() {
    let (mut client, machine, calls, server) = starting_machine().await;
    let started = Instant::now();
    let error = client
        .call_unretried::<op::Initialize>(initialize_request(&machine), None)
        .await
        .unwrap_err();
    assert_eq!(Instant::now() - started, Duration::from_secs(5));
    assert!(
        error
            .to_string()
            .contains("Machine setup mutation reply timed out")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let started = Instant::now();
    let error = client
        .call_repeatable::<op::Inspect>(InspectRequest::default(), None)
        .await
        .unwrap_err();
    assert_eq!(Instant::now() - started, Duration::from_secs(60));
    assert!(error.to_string().contains("Machine setup read timed out"));
    server.abort();
}
