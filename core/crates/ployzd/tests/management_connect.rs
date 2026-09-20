//! Connection contract of the management transport: the public client connector against
//! a real Machine API served over an in-process relay, asserting only what a client sees.

use std::time::Duration;

use iroh::{SecretKey, test_utils::run_relay_server, tls::CaTlsConfig};
use ployz::{
    connect::{ConnectError, Connector, ManagementRelay, SystemConnector},
    context::Connection,
};
use ployz_core::{
    AdvertisedEndpoint, CloudPairing, DescribeContractRequest, InitializeRequest, MachineId,
    MachineName, MachineRpcClient, ManagementCapability, PairingCredential, SetCloudPairingRequest,
    op,
};
use ployzd::{
    machine::{LocalMachine, LocalMachineStore, RecordOwner},
    machine_api::MachineApi,
    management::{self, ManagementConfig},
};
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;

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
    let server = tokio::spawn(management::serve(
        endpoint.clone(),
        owner.watch(),
        MachineApi::builder(owner).build(),
        shutdown.clone(),
    ));
    let connector = SystemConnector::new("/ployz-missing-ssh-client").with_management_relay(
        ManagementRelay::custom(relay_url, CaTlsConfig::insecure_skip_verify()),
    );

    // A key the Machine never accepted is refused by identity, not by reachability.
    let stranger =
        ManagementCapability::new(*capability.machine(), SecretKey::generate().to_bytes());
    assert_refused(connector.connect(&connection(&stranger)).await);

    // Fifty concurrent RPC streams over one accepted connection all complete.
    let channel = connector.connect(&connection(&capability)).await.unwrap();
    let calls: Vec<_> = (0..50)
        .map(|_| tokio::spawn(describe(channel.clone())))
        .collect();
    for call in calls {
        assert_eq!(call.await.unwrap().unwrap(), machine_id);
    }

    // Cancelling in-flight streams and dropping their connection leaves the surviving
    // connection and fresh dials unaffected.
    let cancelled = connector.connect(&connection(&capability)).await.unwrap();
    let in_flight: Vec<_> = (0..50)
        .map(|_| tokio::spawn(describe(cancelled.clone())))
        .collect();
    for call in &in_flight {
        call.abort();
    }
    drop(in_flight);
    drop(cancelled);
    assert_eq!(describe(channel.clone()).await.unwrap(), machine_id);
    let fresh = connector.connect(&connection(&capability)).await.unwrap();
    assert_eq!(describe(fresh).await.unwrap(), machine_id);

    // Clearing the pairing closes the live connection; the old capability is then refused.
    local
        .set_cloud_pairing(SetCloudPairingRequest::Clear {})
        .await
        .unwrap();
    let revoked = async {
        while describe(channel.clone()).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), revoked)
        .await
        .expect("clearing the pairing must close the live connection");
    assert_refused(connector.connect(&connection(&capability)).await);

    // Shutdown drains every remaining connection and closes the endpoint.
    drop(channel);
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
