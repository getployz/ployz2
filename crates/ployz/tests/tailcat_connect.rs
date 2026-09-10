//! Rung 2: real native Tailcat helper, private DERP, and Machine RPC contracts.
//! Requires Go; builds the pinned native prerequisites before exercising RPC.
#![cfg(unix)]

use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use ployz::{
    connect::{SystemConnector, connect_selected_with},
    context::{Connection, ConnectionSource, SelectedConnections},
};
use ployz_core::MachineRpcServer;
use tokio::{io::AsyncBufReadExt, net::UnixListener, process::Command};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

#[allow(dead_code, unused_imports)]
#[path = "connect/support.rs"]
mod support;

#[tokio::test]
async fn native_tailcat_confirms_machine_identity_and_performs_read_only_rpc() {
    let dir = tempfile::tempdir().unwrap();
    let native = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../native/tailcat");
    let helper = dir.path().join("ployz-tailcat");
    let fixture = dir.path().join("rpcfixture");
    assert!(
        Command::new("bash")
            .arg("build.sh")
            .arg(&helper)
            .current_dir(&native)
            .status()
            .await
            .expect("building Tailcat requires bash and Go")
            .success()
    );
    assert!(
        Command::new("go")
            .args(["build", "-o"])
            .arg(&fixture)
            .arg("./testdata/rpcfixture.go")
            .current_dir(&native)
            .status()
            .await
            .expect("building private DERP fixture requires Go")
            .success()
    );
    let socket = dir.path().join("rpc.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let description = support::test_description();
    let service = support::DiscoveryService::new(description.clone());
    let rpc = tokio::spawn(
        Server::builder()
            .add_service(MachineRpcServer::new(service))
            .serve_with_incoming(UnixListenerStream::new(listener)),
    );
    let mut endpoint = Command::new(fixture)
        .arg(&socket)
        .env("IN_TS_TEST", "true")
        // Disable endpoint UDP sockets: every RPC byte must traverse local DERP.
        .env("TS_DEBUG_ALWAYS_USE_DERP", "true")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut output = tokio::io::BufReader::new(endpoint.stdout.take().unwrap()).lines();
    let capability = tokio::time::timeout(Duration::from_secs(20), output.next_line())
        .await
        .expect("private DERP fixture startup timed out")
        .unwrap()
        .expect("private DERP fixture exited before publishing its capability");
    let connector = Arc::new(SystemConnector::default().with_tailcat_program(helper));
    let selected = |connection| SelectedConnections {
        source: ConnectionSource::Direct,
        connections: vec![connection],
    };
    let connection = Connection::tailcat(capability.clone()).unwrap();
    let mut client = connect_selected_with(
        selected(connection.clone().with_machine_id(description.machine_id)),
        connector.clone(),
    )
    .await
    .expect("native Tailcat must complete Machine RPC confirmation");
    assert_eq!(
        client.machines().await.unwrap(),
        vec![support::machine('a', "one")]
    );

    let mismatch = connect_selected_with(
        selected(connection.with_machine_id(support::machine_id('b'))),
        connector.clone(),
    )
    .await;
    assert!(
        mismatch.is_err(),
        "unexpected Machine identity must be rejected"
    );
    let error = mismatch.err().unwrap().to_string();
    assert!(
        error.contains("identity"),
        "identity rejection must identify its cause: {error}"
    );
    assert!(!error.contains(&capability));

    let invalid = "invalid-capability-secret";
    let rejected =
        connect_selected_with(selected(Connection::tailcat(invalid).unwrap()), connector).await;
    assert!(rejected.is_err(), "invalid capability must fail closed");
    assert!(!rejected.err().unwrap().to_string().contains(invalid));
    drop(client);
    endpoint.start_kill().unwrap();
    endpoint.wait().await.unwrap();
    rpc.abort();
}
