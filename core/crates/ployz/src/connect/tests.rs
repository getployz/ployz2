use std::{fs, os::unix::fs::symlink, process::Command as StdCommand, time::Duration};

use super::*;
use crate::context::SshDestination;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn setup_retry_preserves_transient_failures_in_either_connection_order() {
    let program = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ssh");
    for (hosts, retryable) in [
        (["user@transient", "user@permanent"], true),
        (["user@permanent", "user@transient"], true),
        (["user@permanent", "user@permanent"], false),
    ] {
        let selected = SelectedConnections {
            source: ConnectionSource::Context("prod".into()),
            connections: hosts
                .into_iter()
                .map(|host| Connection::ssh(SshDestination::parse(host).unwrap()))
                .collect(),
        };
        let error =
            match connect_selected_with(selected, Arc::new(SystemConnector::new(program))).await {
                Err(error) => error,
                Ok(_) => panic!("script must reject every connection"),
            };
        assert_eq!(error.is_setup_retryable(), retryable, "{error}");
        if !retryable {
            assert!(
                error.to_string().contains("unlock your key with ssh-add"),
                "{error}"
            );
        }
    }
}

#[test]
fn system_ssh_command_uses_noninteractive_authentication() {
    let destination = SshDestination::parse("deploy@example.com:2222").unwrap();

    let args = ssh_args(
        &destination,
        Some(Path::new("/keys/deploy")),
        None,
        SystemConnector::default().ssh_timeout,
    );

    assert_eq!(
        args,
        [
            "-o",
            "ConnectTimeout=5",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-T",
            "-p",
            "2222",
            "-i",
            "/keys/deploy",
            "deploy@example.com",
            "ployzd",
            "dial-stdio",
        ]
    );
}

#[test]
fn transport_predicates_follow_the_grpc_code() {
    #[rustfmt::skip]
    let cases = [
        (tonic::Code::Ok,                 false, false, false, RpcErrorCode::Internal),
        (tonic::Code::Cancelled,          false, false, false, RpcErrorCode::Internal),
        (tonic::Code::Unknown,            false, false, false, RpcErrorCode::Internal),
        (tonic::Code::InvalidArgument,    false, false, false, RpcErrorCode::InvalidArgument),
        (tonic::Code::DeadlineExceeded,   true,  false, false, RpcErrorCode::Unavailable),
        (tonic::Code::NotFound,           false, false, true,  RpcErrorCode::NotFound),
        (tonic::Code::AlreadyExists,      false, false, false, RpcErrorCode::Conflict),
        (tonic::Code::PermissionDenied,   false, false, false, RpcErrorCode::Internal),
        (tonic::Code::ResourceExhausted,  false, false, false, RpcErrorCode::Internal),
        (tonic::Code::FailedPrecondition, false, false, false, RpcErrorCode::Internal),
        (tonic::Code::Aborted,            false, false, false, RpcErrorCode::Conflict),
        (tonic::Code::OutOfRange,         false, false, false, RpcErrorCode::Internal),
        (tonic::Code::Unimplemented,      false, false, false, RpcErrorCode::Unsupported),
        (tonic::Code::Internal,           false, false, false, RpcErrorCode::Internal),
        (tonic::Code::Unavailable,        true,  true,  false, RpcErrorCode::Unavailable),
        (tonic::Code::DataLoss,           false, false, false, RpcErrorCode::Internal),
        (tonic::Code::Unauthenticated,    false, false, false, RpcErrorCode::Unauthenticated),
    ];

    for (code, retryable, unavailable, not_found, rpc) in cases {
        let error = TransportError::from(tonic::Status::new(code, "x"));
        assert_eq!(error.is_retryable(), retryable, "{code:?}");
        assert_eq!(error.is_unavailable(), unavailable, "{code:?}");
        assert_eq!(error.is_not_found(), not_found, "{code:?}");
        assert_eq!(error.to_rpc_error().code, rpc, "{code:?}");
    }
}

#[test]
fn machine_rpc_status_prints_the_message_not_transport_metadata() {
    let status = tonic::Status::invalid_argument("invalid log time \"notatime\"");
    let leaked = status.to_string();
    assert!(leaked.contains("MetadataMap"), "{leaked}");
    assert!(leaked.contains("InvalidArgument"), "{leaked}");

    let error = TransportError::from(status);
    assert_eq!(error.to_string(), "invalid log time \"notatime\"");
    assert_eq!(
        ConnectError::Rpc(error).to_string(),
        "Machine RPC failed: invalid log time \"notatime\""
    );
    assert_eq!(
        crate::operator::LogError::from(tonic::Status::invalid_argument(
            "invalid log time \"notatime\""
        ))
        .to_string(),
        "invalid log time \"notatime\""
    );
}

#[test]
fn only_transport_unavailability_is_an_unreachable_fallback() {
    let deadline = ConnectError::Rpc(TransportError::from(tonic::Status::deadline_exceeded(
        "timed out",
    )));
    assert!(deadline.is_retryable());
    for (error, unreachable) in [
        (
            ConnectError::Rpc(TransportError::from(tonic::Status::unavailable(
                "route failed",
            ))),
            true,
        ),
        (
            ConnectError::Rpc(TransportError::from(tonic::Status::unimplemented(
                "older daemon",
            ))),
            false,
        ),
        (
            ConnectError::Remote(RpcError {
                code: RpcErrorCode::Unavailable,
                message: "Docker is unavailable".into(),
                details: serde_json::Value::Null,
            }),
            false,
        ),
        (deadline, false),
    ] {
        assert_eq!(error.is_unreachable(), unreachable, "{error}");
    }
}

#[tokio::test]
async fn cancelling_ssh_establishment_returns_without_waiting_for_ssh() {
    let root = std::env::temp_dir().join(format!("ployz-ssh-cancel-{}", std::process::id()));
    let program = root.join("ssh");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    symlink(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ssh"),
        &program,
    )
    .unwrap();
    let connector = SystemConnector::new(&program);
    let connection = Connection::ssh(SshDestination::parse("user@cancel").unwrap());

    let pid = tokio::time::timeout(Duration::from_secs(5), async {
        // Finishing the readiness branch drops the pending connection and its SSH child.
        tokio::select! {
            result = connector.connect(&connection) => panic!("SSH finished before cancellation: {result:?}"),
            pid = async {
                loop {
                    if let Ok(contents) = fs::read_to_string(program.with_extension("pid"))
                        && let Ok(pid) = contents.trim().parse::<u32>()
                    {
                        break pid;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            } => pid,
        }
    })
    .await
    .expect("fake SSH did not publish its PID");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while StdCommand::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
    {
        assert!(tokio::time::Instant::now() < deadline, "ssh was not killed");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn missing_ssh_client_survives_connection_selection() {
    let error = match connect_selected_with(
        SelectedConnections {
            source: ConnectionSource::Direct,
            connections: vec![Connection::ssh(
                SshDestination::parse("user@example.com").unwrap(),
            )],
        },
        Arc::new(SystemConnector::new("/ployz-missing-ssh-client")),
    )
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("missing ssh program must fail"),
    };
    assert!(
        matches!(error, ConnectError::SshClientMissing(_)),
        "{error:?}"
    );
    let failure = crate::failure::Failure::from(error);
    assert_eq!(
        failure.to_string(),
        "local ssh client not found; install an ssh client"
    );
    assert!(!failure.to_string().contains("os error"), "{failure}");
}

#[tokio::test]
async fn missing_ssh_client_does_not_skip_later_non_ssh_connections() {
    let error = connect_selected_with(
        SelectedConnections {
            source: ConnectionSource::Direct,
            connections: vec![
                Connection::ssh(SshDestination::parse("user@example.com").unwrap()),
                Connection::unix("/ployz-missing-rpc.sock").unwrap(),
            ],
        },
        Arc::new(SystemConnector::new("/ployz-missing-ssh-client")),
    )
    .await
    .err()
    .expect("both unavailable connections must fail");
    assert!(
        matches!(error, ConnectError::AllFailed { attempts: 2, .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn stalled_ssh_probe_obeys_configured_timeout() {
    let program = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ssh");
    let connector = SystemConnector::new(program).with_ssh_timeout(Duration::from_millis(100));
    let connection = Connection::ssh(SshDestination::parse("user@timeout").unwrap());
    let result = tokio::time::timeout(Duration::from_secs(2), connector.connect(&connection))
        .await
        .expect("SSH setup must stop at its own deadline");
    assert!(
        matches!(result, Err(ConnectError::Io(ref error)) if error.kind() == io::ErrorKind::TimedOut),
        "{result:?}"
    );
}

#[test]
fn ssh_timeout_flag_is_global_and_reaches_transport_arguments() {
    for (args, seconds) in [
        (vec!["ployz", "ps"], 5),
        (vec!["ployz", "--ssh-timeout", "17", "ps"], 17),
        (vec!["ployz", "ps", "--ssh-timeout", "17"], 17),
        (vec!["ployz", "machine", "ls", "--ssh-timeout", "17"], 17),
    ] {
        let root = crate::cli::command().try_get_matches_from(args).unwrap();
        let mut matches = &root;
        loop {
            let timeout = crate::cli::ssh_timeout(matches);
            assert_eq!(timeout, Duration::from_secs(seconds));
            let connector = SystemConnector::default().with_ssh_timeout(timeout);
            let args = ssh_args(
                &SshDestination::parse("user@host").unwrap(),
                None,
                None,
                connector.ssh_timeout,
            );
            assert!(args.contains(&format!("ConnectTimeout={seconds}")));
            let Some((_, child)) = matches.subcommand() else {
                break;
            };
            matches = child;
        }
    }
    for value in ["0", "-1", "abc", "1.5", "4294967296"] {
        assert!(
            crate::cli::command()
                .try_get_matches_from(["ployz", "ls", "--ssh-timeout", value])
                .is_err()
        );
    }
}

#[tokio::test]
async fn management_auxiliary_proxy_is_explicitly_unsupported_and_redacted() {
    let secret = ployz_core::ManagementCapability::new(
        ployz_core::ManagementIdentity::from_bytes([1; 32]),
        [2; 32],
    )
    .to_secret_string();
    let connection = Connection::management(&secret).unwrap();
    let result = SystemConnector::default()
        .dial_proxy(&connection, "tcp", "127.0.0.1:1234")
        .await;
    let Err(ConnectError::ProxyUnsupported(message)) = result else {
        panic!("the management transport must reject auxiliary proxy");
    };
    assert!(message.contains("management"));
    assert!(!message.contains(&secret));
}

#[tokio::test]
async fn child_stream_preserves_half_close_and_reaps_on_cancellation() {
    use tokio::io::AsyncReadExt;
    let mut stream = spawn_child(
        Path::new("sh"),
        &["-c".into(), "cat; printf response-after-eof".into()],
    )
    .unwrap();
    stream.write_all(b"request").await.unwrap();
    stream.shutdown().await.unwrap();
    let mut result = String::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_string(&mut result))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, "requestresponse-after-eof");
    assert!(stream._child.wait().await.unwrap().success());

    let child = spawn_child(Path::new("cat"), &[]).unwrap();
    let pid = child._child.id().unwrap();
    drop(child);
    tokio::time::timeout(Duration::from_secs(3), async {
        while StdCommand::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cancelled helper was not reaped");
}
