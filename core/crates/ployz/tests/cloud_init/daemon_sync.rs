//! Daemon version synchronization before the Cloud enrollment exchange.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use clap::ArgMatches;
use serde_json::json;

use super::harness::{
    EnrollListen, JoinDaemon, PAIRING, TOKEN, registration, serve_local_machine, serve_machine,
};

#[derive(Clone, Copy)]
enum InstallOutcome {
    UpdateDaemon,
    LeaveStale,
    Fail(&'static str),
}

/// A stand-in installer that counts calls into `calls` instead of provisioning.
fn recording_installer(
    daemon: JoinDaemon,
    outcome: InstallOutcome,
    calls: Arc<AtomicUsize>,
) -> impl Fn(ployz_core::StorageChoice) -> std::future::Ready<Result<(), ployz::handlers::Error>> {
    move |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        std::future::ready(match outcome {
            InstallOutcome::UpdateDaemon => {
                daemon.set_daemon_version(env!("CARGO_PKG_VERSION"));
                Ok(())
            }
            InstallOutcome::LeaveStale => Ok(()),
            InstallOutcome::Fail(message) => Err(ployz::handlers::Error::usage(message)),
        })
    }
}

fn enroll_matches(connect: &str, cloud_url: &str) -> ArgMatches {
    ployz::cli::command()
        .try_get_matches_from([
            "ployz",
            "--connect",
            connect,
            "cloud",
            "enroll",
            TOKEN,
            "--cloud-url",
            cloud_url,
            "--name",
            "joiner",
            "--accepts-ingress=false",
            "--no-dns",
            "--yes",
        ])
        .unwrap()
}

async fn run_enroll(
    matches: ArgMatches,
    install: impl Fn(
        ployz_core::StorageChoice,
    ) -> std::future::Ready<Result<(), ployz::handlers::Error>>
    + Send
    + 'static,
) -> Result<(), ployz::handlers::Error> {
    tokio::task::spawn_blocking(move || {
        ployz::handlers::cloud_enroll_with_installer(&matches, &install)
    })
    .await
    .unwrap()
}

struct LocalEnroll {
    result: Result<(), ployz::handlers::Error>,
    calls: usize,
    connections: usize,
    posts: usize,
}

async fn enroll_locally(
    daemon_version: &str,
    outcome: InstallOutcome,
    storage: &str,
) -> LocalEnroll {
    let mut registration = registration();
    registration.assigned_machine.accepts_ingress = false;
    let pairing = json!({ "secret": PAIRING });
    let enroll = EnrollListen::start(json!({
        "kind": "join",
        "storage": storage,
        "pairing": pairing,
        "registration": registration,
    }))
    .await;
    let daemon = JoinDaemon::new(registration);
    daemon.set_daemon_version(daemon_version);
    let (connect, socket, connections) = serve_local_machine(daemon.clone()).await;
    let calls = Arc::<AtomicUsize>::default();
    let installer = recording_installer(daemon.clone(), outcome, Arc::clone(&calls));
    let result = run_enroll(enroll_matches(&connect, &enroll.url), installer).await;
    let _ = std::fs::remove_file(socket);
    LocalEnroll {
        result,
        calls: calls.load(Ordering::SeqCst),
        connections: connections.load(Ordering::SeqCst),
        posts: enroll.posts().len(),
    }
}

#[tokio::test]
async fn zfs_preparation_reconnects_after_restarting_a_matching_daemon() {
    let enrolled = enroll_locally(
        env!("CARGO_PKG_VERSION"),
        InstallOutcome::UpdateDaemon,
        "zfs",
    )
    .await;

    assert!(enrolled.result.is_ok(), "{:?}", enrolled.result);
    assert_eq!(enrolled.calls, 1);
    assert!(
        enrolled.connections >= 2,
        "enrollment must reconnect after storage preparation restarts the daemon"
    );
}

#[tokio::test]
async fn local_daemon_synchronization() {
    let current = env!("CARGO_PKG_VERSION");
    let stale = format!("daemon version remained 0.0.0-old after installing CLI version {current}");
    // (daemon version, installer outcome) -> (installer calls, error, reconnected)
    let rows: [(&str, InstallOutcome, usize, Option<&str>, bool); 4] = [
        (current, InstallOutcome::UpdateDaemon, 0, None, false),
        ("0.0.0-old", InstallOutcome::UpdateDaemon, 1, None, true),
        (
            "0.0.0-old",
            InstallOutcome::Fail("installer failed"),
            1,
            Some("installer failed"),
            false,
        ),
        (
            "0.0.0-old",
            InstallOutcome::LeaveStale,
            1,
            Some(stale.as_str()),
            false,
        ),
    ];
    for (version, outcome, calls, error, reconnected) in rows {
        let enrolled = enroll_locally(version, outcome, "none").await;
        let row = format!("daemon {version}");
        assert_eq!(enrolled.calls, calls, "{row}");
        match error {
            None => {
                assert!(enrolled.result.is_ok(), "{row}: {:?}", enrolled.result);
                assert_eq!(enrolled.posts, 1, "{row}");
            }
            Some(error) => {
                assert_eq!(enrolled.result.unwrap_err().to_string(), error, "{row}");
                assert_eq!(enrolled.posts, 0, "{row}: failed sync must not enroll");
            }
        }
        if reconnected {
            assert!(
                enrolled.connections >= 2,
                "{row}: enrollment must reconnect after the installer restarts the daemon"
            );
        }
    }
}

#[tokio::test]
async fn remote_mismatch_is_rejected_without_mutating_the_local_machine() {
    let registration = registration();
    let daemon = JoinDaemon::new(registration);
    daemon.set_daemon_version("0.0.0-old");
    let address = serve_machine(daemon.clone()).await;
    let connect = format!("tcp://{address}");
    let calls = Arc::<AtomicUsize>::default();
    let installer = recording_installer(daemon, InstallOutcome::UpdateDaemon, Arc::clone(&calls));
    let result = run_enroll(enroll_matches(&connect, "http://127.0.0.1:9"), installer).await;

    assert_eq!(
        result.unwrap_err().to_string(),
        format!(
            "daemon version synchronization requires running ployz cloud enroll on the Machine itself; connected through {connect}"
        )
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
