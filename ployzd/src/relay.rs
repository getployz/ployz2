//! Cloud Relay Register client.

use std::{io, time::Duration};

use ployz_core::{CloudPairing, MachineId};
use ployz_relay::{AUTHORIZATION_METADATA, CloudRelayClient, PairingCredential, RegisterRequest};
use thiserror::Error;
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};
use tokio_stream::{StreamExt, wrappers::ReceiverStream};
use tokio_util::sync::CancellationToken;
use tonic::{
    Request,
    metadata::MetadataValue,
    transport::{Channel, Endpoint},
};

/// Held Cloud Relay Register. Dropping it ends the stream.
#[must_use]
pub struct RegisterHold {
    task: JoinHandle<()>,
}

/// Failures holding Cloud Relay Register.
#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Transport(#[from] tonic::transport::Error),
    #[error(transparent)]
    Status(#[from] tonic::Status),
}

/// Hold Register: hello with Machine ID, Pairing Credential bearer, echo pings.
///
/// # Errors
///
/// If the Relay is unreachable or Register is rejected.
pub async fn hold_register(
    url: &str,
    pairing: &PairingCredential,
    machine_id: &MachineId,
) -> Result<RegisterHold, Error> {
    let mut relay = CloudRelayClient::new(connect(url).await?);
    let (tx, rx) = mpsc::channel(4);
    tx.send(RegisterRequest::new(machine_id))
        .await
        .expect("hello is sent before the hold task starts");
    let mut request = Request::new(ReceiverStream::new(rx));
    set_bearer(request.metadata_mut(), pairing);
    let mut opens = relay.register(request).await?.into_inner();
    let task = tokio::spawn(async move {
        while let Some(Ok(message)) = opens.next().await {
            // ponytail: Attach is a later ticket
            if let Some(nonce) = message.ping_nonce() {
                let _ = tx.send(RegisterRequest::pong(nonce)).await;
            }
        }
    });
    Ok(RegisterHold { task })
}

impl Drop for RegisterHold {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Hold Cloud Relay Register after this Machine is participating.
///
/// No Cloud Pairing is a no-op: SSH-only Machines stay off List.
///
/// # Errors
///
/// If participation wait fails, or Register is rejected after participation.
pub async fn hold_cloud_pairing(
    pairing: Option<CloudPairing>,
    machine_id: MachineId,
    mut participating: watch::Receiver<bool>,
    shutdown: CancellationToken,
) -> io::Result<()> {
    let Some(pairing) = pairing else {
        shutdown.cancelled().await;
        return Ok(());
    };
    tokio::select! {
        biased;
        () = shutdown.cancelled() => return Ok(()),
        changed = participating.wait_for(|participating| *participating) => {
            match changed {
                Ok(_) if shutdown.is_cancelled() => return Ok(()),
                Ok(_) => {}
                Err(_) if shutdown.is_cancelled() => return Ok(()),
                Err(error) => return Err(io::Error::other(error)),
            }
        }
    }
    let secret = PairingCredential::parse(pairing.secret().as_str())
        .expect("Cloud Pairing secret is a non-empty Pairing Credential");
    let _hold = hold_register(pairing.relay_url(), &secret, &machine_id)
        .await
        .map_err(io::Error::other)?;
    shutdown.cancelled().await;
    Ok(())
}

async fn connect(url: &str) -> Result<Channel, tonic::transport::Error> {
    Endpoint::from_shared(url.to_owned())?
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
}

fn set_bearer(metadata: &mut tonic::metadata::MetadataMap, pairing: &PairingCredential) {
    metadata.insert(
        AUTHORIZATION_METADATA,
        MetadataValue::try_from(format!("Bearer {}", pairing.as_str()))
            .expect("Pairing Credential is ASCII metadata"),
    );
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        net::Ipv4Addr,
        path::PathBuf,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use ployz_core::{
        AdvertisedEndpoint, CloudPairing, InitializeRequest, JoinRequest, Machine, MachineId,
        MachineName, Registered,
    };
    use ployz_relay::{
        AUTHORIZATION_METADATA, CloudRelayClient, DialCredential, ListRequest, MACHINE_ID_METADATA,
        PAIRING_METADATA, PairingCredential, Relay, RevokeRequest,
    };
    use tokio::sync::{mpsc, watch};
    use tokio_stream::wrappers::ReceiverStream;
    use tokio_util::sync::CancellationToken;
    use tonic::{Request, metadata::MetadataValue, transport::Endpoint};

    use super::{Error, hold_cloud_pairing, hold_register};
    use crate::{
        machine::{LocalMachine, LocalMachineStore},
        network::management_address,
    };

    const PAIRING: &str = "pairing-secret";
    const DIAL: &str = "dial-secret";
    const OTHER: &str = "other-pairing";

    #[tokio::test]
    async fn register_hello_appears_on_list_held() {
        let session = Session::start().await;
        let listed = list(&session.url, DIAL, PAIRING).await.unwrap();
        let [row] = listed.as_slice() else {
            panic!("expected one held Register, got {listed:?}");
        };
        assert_eq!(row.machine_id().unwrap(), session.machine_id);
    }

    #[tokio::test]
    async fn register_pings_are_echoed() {
        let session = Session::start().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let listed = list(&session.url, DIAL, PAIRING).await.unwrap();
            if listed.iter().any(|row| {
                row.machine_id().ok() == Some(session.machine_id) && row.register_rtt_ns.is_some()
            }) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "path RTT must be present after a pong"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn pairing_credential_cannot_impersonate_cloud() {
        let session = Session::start().await;
        let mut cloud = client(&session.url).await;

        let list = cloud
            .list(cloud_request(ListRequest {}, PAIRING, PAIRING, None))
            .await;
        assert_eq!(list.unwrap_err().code(), tonic::Code::Unauthenticated);

        let revoke = cloud
            .revoke(cloud_request(RevokeRequest {}, PAIRING, PAIRING, None))
            .await;
        assert_eq!(revoke.unwrap_err().code(), tonic::Code::Unauthenticated);

        let dial = cloud
            .dial(cloud_request(
                ReceiverStream::new(mpsc::channel(1).1),
                PAIRING,
                PAIRING,
                Some(&session.machine_id),
            ))
            .await;
        assert_eq!(dial.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn wrong_pairing_does_not_list_this_machine() {
        let session = Session::start().await;
        let listed = list(&session.url, DIAL, OTHER).await.unwrap();
        assert!(
            listed
                .iter()
                .all(|row| row.machine_id().ok() != Some(session.machine_id))
        );
    }

    #[tokio::test]
    async fn unreachable_relay_fails() {
        let pairing = PairingCredential::parse(PAIRING).unwrap();
        let error = match hold_register("not-a-url", &pairing, &MachineId::random()).await {
            Ok(_) => panic!("expected unreachable Relay to fail"),
            Err(error) => error,
        };
        assert!(matches!(error, Error::Transport(_)));
    }

    #[tokio::test]
    async fn initialize_with_cloud_pairing_appears_on_list_held() {
        let relay = RelayListen::start().await;
        let dir = TestDir::new("ployzd-relay-initialize");
        let local = local_machine(&dir.0);
        let pairing = cloud_pairing(&relay.url);
        let initialized = local
            .initialize(InitializeRequest {
                name: MachineName::parse("first").unwrap(),
                cluster_network: "10.210.0.0/16".parse().unwrap(),
                public_ip: None,
                advertised_endpoints: endpoint(),
                wireguard_mtu: None,
                cloud_pairing: Some(pairing.clone()),
            })
            .unwrap();
        let record = local.record().unwrap();
        assert_eq!(record.cloud_pairing.as_ref(), Some(&pairing));

        let shutdown = CancellationToken::new();
        let hold = tokio::spawn(hold_cloud_pairing(
            record.cloud_pairing.clone(),
            initialized.machine.id,
            watch::channel(true).1,
            shutdown.clone(),
        ));
        wait_for_held(&relay.url, initialized.machine.id, true).await;
        shutdown.cancel();
        hold.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn join_with_cloud_pairing_appears_on_list_held_after_catch_up() {
        let relay = RelayListen::start().await;
        let first_dir = TestDir::new("ployzd-relay-join-first");
        let mut first = LocalMachineStore::open(&first_dir.0).unwrap();
        let peer = first
            .initialize(
                MachineName::parse("first").unwrap(),
                "10.210.0.0/16".parse().unwrap(),
                None,
                endpoint(),
                None,
                None,
            )
            .unwrap();

        let second_dir = TestDir::new("ployzd-relay-join-second");
        let store = LocalMachineStore::open(&second_dir.0).unwrap();
        let public_key = store.record().wireguard_private_key.public_key();
        let assigned = Machine {
            id: MachineId::random(),
            name: MachineName::parse("second").unwrap(),
            subnet: "10.210.1.0/24".parse().unwrap(),
            management_address: management_address(public_key),
            public_key,
            public_ip: None,
            advertised_endpoints: vec![AdvertisedEndpoint("192.0.2.2:51820".parse().unwrap())],
            runtime: Default::default(),
        };
        let pairing = cloud_pairing(&relay.url);
        let local = LocalMachine::new(Arc::new(Mutex::new(store)), watch::channel(false).0);
        local
            .join(JoinRequest {
                registration: Registered {
                    assigned_machine: assigned.clone(),
                    visible_peers: vec![peer],
                    target_versions: BTreeMap::from([("actor".into(), 4)]),
                },
                wireguard_mtu: None,
                cloud_pairing: Some(pairing.clone()),
            })
            .unwrap();
        drop(local);

        let (participating, participating_rx) = watch::channel(false);
        let shutdown = CancellationToken::new();
        let hold = tokio::spawn(hold_cloud_pairing(
            Some(pairing),
            assigned.id,
            participating_rx,
            shutdown.clone(),
        ));
        assert_not_held(&relay.url, assigned.id).await;

        let mut store = LocalMachineStore::open(&second_dir.0).unwrap();
        store.complete_catch_up().unwrap();
        participating.send_replace(true);
        wait_for_held(&relay.url, assigned.id, true).await;
        shutdown.cancel();
        hold.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn ssh_only_initialize_stays_off_list_held() {
        let relay = RelayListen::start().await;
        let dir = TestDir::new("ployzd-relay-ssh-only");
        let local = local_machine(&dir.0);
        let initialized = local
            .initialize(InitializeRequest {
                name: MachineName::parse("ssh").unwrap(),
                cluster_network: "10.210.0.0/16".parse().unwrap(),
                public_ip: None,
                advertised_endpoints: endpoint(),
                wireguard_mtu: None,
                cloud_pairing: None,
            })
            .unwrap();
        assert_eq!(local.record().unwrap().cloud_pairing, None);

        let shutdown = CancellationToken::new();
        let hold = tokio::spawn(hold_cloud_pairing(
            None,
            initialized.machine.id,
            watch::channel(true).1,
            shutdown.clone(),
        ));
        assert_not_held(&relay.url, initialized.machine.id).await;
        shutdown.cancel();
        hold.await.unwrap().unwrap();
    }

    struct RelayListen {
        url: String,
        _server: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
        _goaway: ployz_relay::Goaway,
    }

    impl RelayListen {
        async fn start() -> Self {
            let relay = Relay::new(DialCredential::parse(DIAL).unwrap());
            let listen = (Ipv4Addr::LOCALHOST, 0).into();
            let (address, server, goaway) = relay.serve(listen).await.unwrap();
            Self {
                url: format!("http://{address}"),
                _server: server,
                _goaway: goaway,
            }
        }
    }

    struct Session {
        url: String,
        machine_id: MachineId,
        _hold: super::RegisterHold,
        _relay: RelayListen,
    }

    impl Session {
        async fn start() -> Self {
            let relay = RelayListen::start().await;
            let machine_id = MachineId::random();
            let pairing = PairingCredential::parse(PAIRING).unwrap();
            let hold = hold_register(&relay.url, &pairing, &machine_id)
                .await
                .expect("Register hello is accepted");
            Self {
                url: relay.url.clone(),
                machine_id,
                _hold: hold,
                _relay: relay,
            }
        }
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(prefix: &str) -> Self {
            Self(std::env::temp_dir().join(format!("{prefix}-{}", MachineId::random())))
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn local_machine(data_dir: &std::path::Path) -> LocalMachine {
        LocalMachine::new(
            Arc::new(Mutex::new(LocalMachineStore::open(data_dir).unwrap())),
            watch::channel(false).0,
        )
    }

    fn cloud_pairing(relay_url: &str) -> CloudPairing {
        CloudPairing::parse(
            relay_url,
            ployz_core::PairingCredential::parse(PAIRING).unwrap(),
        )
        .unwrap()
    }

    fn endpoint() -> Vec<AdvertisedEndpoint> {
        vec![AdvertisedEndpoint("192.0.2.1:51820".parse().unwrap())]
    }

    async fn wait_for_held(url: &str, machine_id: MachineId, want_rtt: bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let listed = list(url, DIAL, PAIRING).await.unwrap();
            if listed.iter().any(|row| {
                row.machine_id().ok() == Some(machine_id)
                    && (!want_rtt || row.register_rtt_ns.is_some())
            }) {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "expected {machine_id} on List, got {listed:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn assert_not_held(url: &str, machine_id: MachineId) {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let listed = list(url, DIAL, PAIRING).await.unwrap();
        assert!(
            listed
                .iter()
                .all(|row| row.machine_id().ok() != Some(machine_id)),
            "Machine must stay off List, got {listed:?}"
        );
    }

    async fn list(
        url: &str,
        dial: &str,
        pairing: &str,
    ) -> Result<Vec<ployz_relay::HeldRegister>, tonic::Status> {
        client(url)
            .await
            .list(cloud_request(ListRequest {}, dial, pairing, None))
            .await
            .map(|response| response.into_inner().registers().to_vec())
    }

    async fn client(url: &str) -> CloudRelayClient<tonic::transport::Channel> {
        let channel = Endpoint::from_shared(url.to_owned())
            .unwrap()
            .connect_timeout(Duration::from_secs(5))
            .connect()
            .await
            .unwrap();
        CloudRelayClient::new(channel)
    }

    fn cloud_request<T>(
        body: T,
        bearer: &str,
        pairing: &str,
        machine_id: Option<&MachineId>,
    ) -> Request<T> {
        let mut request = Request::new(body);
        request.metadata_mut().insert(
            AUTHORIZATION_METADATA,
            MetadataValue::try_from(format!("Bearer {bearer}")).expect("bearer is ASCII"),
        );
        request.metadata_mut().insert(
            PAIRING_METADATA,
            pairing.parse().expect("pairing is ASCII metadata"),
        );
        if let Some(machine_id) = machine_id {
            request.metadata_mut().insert(
                MACHINE_ID_METADATA,
                machine_id
                    .as_str()
                    .parse()
                    .expect("Machine ID is ASCII metadata"),
            );
        }
        request
    }
}
