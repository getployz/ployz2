use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use ployz::connect::{ConnectError, DialCredential, connect_relay};
use ployz_core::{DescribeContractRequest, MachineId, MachineRpcServer, op};
use ployz_relay::{
    AUTHORIZATION_METADATA, CloudRelayClient, PairingCredential, RegisterRequest, Relay,
    TUNNEL_ID_METADATA, TunnelFrame,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf},
    sync::mpsc,
    time::timeout,
};
use tokio_stream::{StreamExt, wrappers::ReceiverStream};
use tonic::{
    Request,
    metadata::MetadataValue,
    transport::{Endpoint, server::Connected},
};

use super::support::{DiscoveryService, test_description};

const PAIRING: &str = "pairing-secret";
const DIAL: &str = "dial-secret";

#[tokio::test]
async fn client_rpc_round_trip_through_relay_attach() {
    let description = test_description();
    let machine_id = description.machine_id;
    let session = RelaySession::start().await;
    let _machine = session
        .spawn_machine(machine_id, DiscoveryService::new(description.clone()))
        .await;

    let mut client = connect_relay(&session.url, dial_credential(), machine_id)
        .await
        .unwrap();

    assert_eq!(
        client
            .call::<op::DescribeContract>(DescribeContractRequest {}, None)
            .await
            .unwrap(),
        description
    );
}

#[tokio::test]
async fn bad_dial_credential_fails_closed() {
    let machine_id = MachineId::random();
    let session = RelaySession::start().await;
    let _machine = session
        .spawn_machine(machine_id, DiscoveryService::new(test_description()))
        .await;
    let bad = DialCredential::parse("wrong-secret").unwrap();

    let result = timeout(
        Duration::from_secs(2),
        connect_relay(&session.url, bad, machine_id),
    )
    .await
    .expect("bad Dial Credential must not hang");
    let error = match result {
        Ok(_) => panic!("expected invalid Dial Credential to fail"),
        Err(error) => error,
    };

    assert!(
        matches!(error, ConnectError::InvalidDialCredential),
        "{error:?}"
    );
}

#[tokio::test]
async fn unknown_machine_id_fails_closed() {
    let registered = MachineId::random();
    let session = RelaySession::start().await;
    let _machine = session
        .spawn_machine(registered, DiscoveryService::new(test_description()))
        .await;

    let result = timeout(
        Duration::from_secs(2),
        connect_relay(&session.url, dial_credential(), MachineId::random()),
    )
    .await
    .expect("unknown Machine ID must not hang");
    let error = match result {
        Ok(_) => panic!("expected unknown Machine ID to fail"),
        Err(error) => error,
    };

    assert!(matches!(error, ConnectError::UnknownMachine), "{error:?}");
}

struct RelaySession {
    url: String,
    _server: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
}

impl RelaySession {
    async fn start() -> Self {
        let relay = Relay::new(
            PairingCredential::parse(PAIRING).unwrap(),
            DialCredential::parse(DIAL).unwrap(),
        )
        .unwrap();
        let (address, server) = relay.serve().await.unwrap();
        Self {
            url: format!("http://{address}"),
            _server: server,
        }
    }

    async fn spawn_machine(&self, machine_id: MachineId, service: DiscoveryService) -> FakeMachine {
        FakeMachine::register(&self.url, machine_id, service).await
    }
}

struct FakeMachine {
    _register_hold: mpsc::Sender<RegisterRequest>,
    _accept: tokio::task::JoinHandle<()>,
}

impl FakeMachine {
    async fn register(url: &str, machine_id: MachineId, service: DiscoveryService) -> Self {
        let mut relay = relay_client(url).await;
        let (hold, rx) = mpsc::channel(4);
        hold.send(RegisterRequest::new(&machine_id)).await.unwrap();
        let mut request = Request::new(ReceiverStream::new(rx));
        set_bearer(request.metadata_mut(), PAIRING);
        let mut opens = relay.register(request).await.unwrap().into_inner();
        let accept = tokio::spawn(async move {
            while let Some(Ok(open)) = opens.next().await {
                let mut attach_client = relay.clone();
                let service = service.clone();
                tokio::spawn(async move {
                    serve_attach(&mut attach_client, open, service).await;
                });
            }
        });
        Self {
            _register_hold: hold,
            _accept: accept,
        }
    }
}

async fn serve_attach(
    relay: &mut CloudRelayClient<tonic::transport::Channel>,
    open: ployz_relay::Open,
    service: DiscoveryService,
) {
    let (tx, rx) = mpsc::channel(16);
    let mut request = Request::new(ReceiverStream::new(rx));
    request.metadata_mut().insert(
        TUNNEL_ID_METADATA,
        open.tunnel_id()
            .expect("Open carries a Tunnel ID")
            .as_str()
            .parse()
            .expect("Tunnel ID is ASCII metadata"),
    );
    let inbound = relay.attach(request).await.unwrap().into_inner();
    let io = tunnel_io(tx, inbound);
    let _ = tonic::transport::Server::builder()
        .add_service(MachineRpcServer::new(service))
        .serve_with_incoming(tokio_stream::once(Ok::<_, io::Error>(io)))
        .await;
}

fn tunnel_io(tx: mpsc::Sender<TunnelFrame>, inbound: tonic::Streaming<TunnelFrame>) -> AttachIo {
    let (io, other) = tokio::io::duplex(64 * 1024);
    tokio::spawn(copy_tunnel(other, tx, inbound));
    AttachIo { inner: io }
}

async fn copy_tunnel(
    io: DuplexStream,
    tx: mpsc::Sender<TunnelFrame>,
    mut inbound: tonic::Streaming<TunnelFrame>,
) {
    let (mut reader, mut writer) = tokio::io::split(io);
    let to_relay = async {
        let mut buf = vec![0_u8; 16 * 1024];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let Some(data) = buf.get(..n) else { break };
                    if tx.send(TunnelFrame::new(data.to_vec())).await.is_err() {
                        break;
                    }
                }
            }
        }
    };
    let from_relay = async {
        while let Some(Ok(frame)) = inbound.next().await {
            if writer.write_all(&frame.data).await.is_err() {
                break;
            }
        }
        let _ = writer.shutdown().await;
    };
    tokio::join!(to_relay, from_relay);
}

struct AttachIo {
    inner: DuplexStream,
}

impl Connected for AttachIo {
    type ConnectInfo = ();

    fn connect_info(&self) -> Self::ConnectInfo {}
}

impl AsyncRead for AttachIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(context, buffer)
    }
}

impl AsyncWrite for AttachIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(context, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }
}

async fn relay_client(url: &str) -> CloudRelayClient<tonic::transport::Channel> {
    let channel = Endpoint::from_shared(url.to_owned())
        .unwrap()
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
        .unwrap();
    CloudRelayClient::new(channel)
}

fn set_bearer(metadata: &mut tonic::metadata::MetadataMap, credential: &str) {
    metadata.insert(
        AUTHORIZATION_METADATA,
        MetadataValue::try_from(format!("Bearer {credential}")).expect("bearer is ASCII"),
    );
}

fn dial_credential() -> DialCredential {
    DialCredential::parse(DIAL).unwrap()
}
