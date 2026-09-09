//! Durable, bounded Machine upgrade attempts run by transient systemd services.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use ployz_core::{
    MachineRelease, MachineUpgradeAttempt, MachineUpgradeAttemptId, MachineUpgradeStage,
    MachineVersion, RequestMachineUpgradeRequest,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{process::Command, time::timeout};

use super::{
    Error as InstallError, InstallPaths, InstallRequest, InstallStage, Preparation, ReleaseRequest,
    ReleaseSource, admission,
};

const RECEIPT_FILE: &str = "upgrade-attempt.json";
const QUALIFICATION_RELEASE_DIR: &str = "PLOYZ_UPGRADE_RELEASE_DIR";
const WORKER_RUNTIME: &str = "15min";
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Error)]
pub enum Error {
    #[error("no Machine upgrade attempt was found")]
    NotFound,
    #[error("Machine upgrade attempt {0} was reused with a different release")]
    AttemptConflict(MachineUpgradeAttemptId),
    #[error("a Machine upgrade or mutation is active")]
    Busy,
    #[error("read Machine upgrade receipt: {0}")]
    Read(#[source] io::Error),
    #[error("decode Machine upgrade receipt: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("write Machine upgrade receipt: {0}")]
    Write(#[source] io::Error),
    #[error("encode Machine upgrade receipt: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("invalid qualification release directory: {0}")]
    QualificationSource(String),
    #[error("resolve Machine release: {0}")]
    Resolve(#[source] InstallError),
    #[error("launch Machine upgrade worker: {0}")]
    Launch(String),
    #[error("inspect Machine upgrade worker: {0}")]
    InspectWorker(#[source] io::Error),
    #[error("Machine upgrade worker does not own active attempt {0}")]
    NotActive(MachineUpgradeAttemptId),
    #[error("Machine upgrade failed: {0}")]
    Installation(#[source] InstallError),
    #[error(transparent)]
    Admission(#[from] admission::Error),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct StoredAttempt {
    requested: MachineRelease,
    source: StoredReleaseSource,
    attempt: MachineUpgradeAttempt,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StoredReleaseSource {
    Published,
    Local { directory: PathBuf },
}

impl StoredReleaseSource {
    fn current() -> Result<Self, Error> {
        let Some(directory) = env::var_os(QUALIFICATION_RELEASE_DIR) else {
            return Ok(Self::Published);
        };
        let directory = PathBuf::from(directory);
        if !directory.is_absolute() {
            return Err(Error::QualificationSource(format!(
                "{QUALIFICATION_RELEASE_DIR} must be an absolute path"
            )));
        }
        Ok(Self::Local { directory })
    }

    fn installer(&self) -> ReleaseSource {
        match self {
            Self::Published => ReleaseSource::Published,
            Self::Local { directory } => ReleaseSource::Local(directory.clone()),
        }
    }
}

/// Return the durable result of retrying the same request without requiring installation
/// ownership. The receipt is atomically replaced, so an active worker can be observed safely.
pub fn existing_request(
    request: &RequestMachineUpgradeRequest,
    data_dir: &Path,
) -> Result<Option<MachineUpgradeAttempt>, Error> {
    let Some(stored) = read_optional(data_dir)? else {
        return Ok(None);
    };
    if stored.attempt.attempt_id() != request.attempt_id {
        return Ok(None);
    }
    if stored.requested != request.release {
        return Err(Error::AttemptConflict(request.attempt_id));
    }
    Ok(Some(stored.attempt))
}

/// Persist an accepted attempt and launch its old-daemon worker while installation admission is
/// held. Reusing the same attempt ID returns the same attempt.
pub async fn request_locked(
    request: RequestMachineUpgradeRequest,
    data_dir: &Path,
    run_dir: &Path,
    _guard: &admission::InstallGuard,
) -> Result<MachineUpgradeAttempt, Error> {
    let admission = admission::Admission::new(run_dir, data_dir);
    if let Some(mut stored) = read_optional(data_dir)? {
        if stored.attempt.attempt_id() == request.attempt_id {
            if stored.requested != request.release {
                return Err(Error::AttemptConflict(request.attempt_id));
            }
            reconcile_locked(&admission, data_dir, &mut stored).await?;
            return Ok(stored.attempt);
        }
        reconcile_locked(&admission, data_dir, &mut stored).await?;
        if !stored.attempt.is_terminal() {
            return Err(Error::Busy);
        }
    }

    let source = StoredReleaseSource::current()?;
    let release = request
        .release
        .as_str()
        .parse::<ReleaseRequest>()
        .map_err(Error::Resolve)?;
    let target = super::release::resolve_release(&release, &source.installer())
        .await
        .map_err(Error::Resolve)?;
    let target = MachineVersion::parse(target.to_string())
        .expect("the installer accepts only supported Machine versions");
    let mut stored = StoredAttempt {
        requested: request.release,
        source,
        attempt: MachineUpgradeAttempt::Accepted {
            attempt_id: request.attempt_id,
            target,
        },
    };
    write(data_dir, &stored)?;
    admission.mark_active(request.attempt_id.as_str())?;

    if let Err(error) = launch_worker(request.attempt_id, data_dir, run_dir).await {
        stored.attempt = MachineUpgradeAttempt::Failed {
            attempt_id: request.attempt_id,
            target: stored.attempt.target().clone(),
            stage: MachineUpgradeStage::Launching,
            error: error.to_string(),
        };
        write(data_dir, &stored)?;
        admission.clear_active(request.attempt_id.as_str())?;
    }
    Ok(stored.attempt)
}

/// Read an attempt, turning a stopped worker without terminal evidence into interruption.
pub async fn inspect(
    attempt_id: Option<MachineUpgradeAttemptId>,
    data_dir: &Path,
    run_dir: &Path,
) -> Result<MachineUpgradeAttempt, Error> {
    let admission = admission::Admission::new(run_dir, data_dir);
    let mut stored = read(data_dir)?;
    if attempt_id.is_some_and(|attempt_id| attempt_id != stored.attempt.attempt_id()) {
        return Err(Error::NotFound);
    }
    if stored.attempt.is_terminal() {
        if let Ok(_guard) = admission.try_install() {
            admission.clear_active(stored.attempt.attempt_id().as_str())?;
        }
        return Ok(stored.attempt);
    }
    if worker_active(stored.attempt.attempt_id()).await? {
        return Ok(stored.attempt);
    }
    let Ok(_guard) = admission.try_install() else {
        return Ok(stored.attempt);
    };
    stored = read(data_dir)?;
    reconcile_locked(&admission, data_dir, &mut stored).await?;
    Ok(stored.attempt)
}

/// Run the accepted attempt from the transient systemd service.
pub async fn run_worker(
    attempt_id: MachineUpgradeAttemptId,
    data_dir: &Path,
    run_dir: &Path,
) -> Result<(), Error> {
    let admission = admission::Admission::new(run_dir, data_dir);
    let guard = admission.lock_install()?;
    let mut stored = read(data_dir)?;
    if stored.attempt.attempt_id() != attempt_id || stored.attempt.is_terminal() {
        return Err(Error::NotActive(attempt_id));
    }
    let target = stored.attempt.target().clone();
    let target_version = Version::parse(target.as_str())
        .expect("a MachineVersion contains a supported semantic version");
    let mut stage = MachineUpgradeStage::Preparing;
    stored.attempt = MachineUpgradeAttempt::Running {
        attempt_id,
        target: target.clone(),
        stage: stage.clone(),
    };
    write(data_dir, &stored)?;

    let result = super::install_locked(
        InstallRequest {
            release: ReleaseRequest::Exact(target_version),
            source: stored.source.installer(),
            preparation: Preparation::SoftwareOnly,
            install_only: false,
        },
        InstallPaths::system(data_dir, run_dir),
        guard,
        |install_stage| {
            stage = upgrade_stage(install_stage);
            stored.attempt = MachineUpgradeAttempt::Running {
                attempt_id,
                target: target.clone(),
                stage: stage.clone(),
            };
            write(data_dir, &stored).map_err(|error| InstallError::Io {
                stage: "record Machine upgrade progress",
                source: io::Error::other(error),
            })
        },
    )
    .await;

    match result {
        Ok(_) => {
            stored.attempt = MachineUpgradeAttempt::Succeeded {
                attempt_id,
                version: target,
            };
            write(data_dir, &stored)?;
            admission.clear_active(attempt_id.as_str())?;
            Ok(())
        }
        Err(error) => {
            stored.attempt = MachineUpgradeAttempt::Failed {
                attempt_id,
                target,
                stage,
                error: error.to_string(),
            };
            write(data_dir, &stored)?;
            admission.clear_active(attempt_id.as_str())?;
            Err(Error::Installation(error))
        }
    }
}

/// Reconcile a retained receipt before serving requests after daemon restart.
pub async fn reconcile(data_dir: &Path, run_dir: &Path) -> Result<(), Error> {
    let admission = admission::Admission::new(run_dir, data_dir);
    let Some(mut stored) = read_optional(data_dir)? else {
        return Ok(());
    };
    if stored.attempt.is_terminal() {
        if let Ok(_guard) = admission.try_install() {
            admission.clear_active(stored.attempt.attempt_id().as_str())?;
        }
        return Ok(());
    }
    let Ok(_guard) = admission.try_install() else {
        return Ok(());
    };
    reconcile_locked(&admission, data_dir, &mut stored).await
}

pub(super) async fn reconcile_for_install(
    admission: &admission::Admission,
    data_dir: &Path,
) -> Result<(), Error> {
    let Some(mut stored) = read_optional(data_dir)? else {
        return Ok(());
    };
    reconcile_locked(admission, data_dir, &mut stored).await?;
    if stored.attempt.is_terminal() {
        Ok(())
    } else {
        Err(Error::Busy)
    }
}

async fn reconcile_locked(
    admission: &admission::Admission,
    data_dir: &Path,
    stored: &mut StoredAttempt,
) -> Result<(), Error> {
    if stored.attempt.is_terminal() {
        admission.clear_active(stored.attempt.attempt_id().as_str())?;
        return Ok(());
    }
    if worker_active(stored.attempt.attempt_id()).await? {
        return Ok(());
    }
    let stage = match &stored.attempt {
        MachineUpgradeAttempt::Accepted { .. } => MachineUpgradeStage::Launching,
        MachineUpgradeAttempt::Running { stage, .. } => stage.clone(),
        MachineUpgradeAttempt::Succeeded { .. }
        | MachineUpgradeAttempt::Failed { .. }
        | MachineUpgradeAttempt::Interrupted { .. } => return Ok(()),
    };
    stored.attempt = MachineUpgradeAttempt::Interrupted {
        attempt_id: stored.attempt.attempt_id(),
        target: stored.attempt.target().clone(),
        stage,
    };
    write(data_dir, stored)?;
    admission.clear_active(stored.attempt.attempt_id().as_str())?;
    Ok(())
}

async fn launch_worker(
    attempt_id: MachineUpgradeAttemptId,
    data_dir: &Path,
    run_dir: &Path,
) -> Result<(), Error> {
    let executable = env::current_exe().map_err(Error::InspectWorker)?;
    let unit = unit_name(attempt_id);
    let writable = format!(
        "ReadWritePaths=/usr/local/bin /etc/systemd/system {} {}",
        data_dir.display(),
        run_dir.display()
    );
    let mut command = Command::new("systemd-run");
    command
        .arg(format!("--unit={unit}"))
        .args(["--collect", "--quiet"])
        .arg("--property=Type=exec")
        .arg(format!("--property=RuntimeMaxSec={WORKER_RUNTIME}"))
        .arg("--property=TimeoutStopSec=15s")
        .arg("--property=KillMode=control-group")
        .arg("--property=NoNewPrivileges=yes")
        .arg("--property=ProtectSystem=full")
        .arg("--property=ProtectHome=yes")
        .arg("--property=ProtectControlGroups=yes")
        .arg("--property=ProtectKernelTunables=yes")
        .arg("--property=PrivateTmp=yes")
        .arg("--property=RestrictNamespaces=yes")
        .arg("--property=RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX")
        .arg(format!("--property={writable}"))
        .arg(executable)
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--socket")
        .arg(run_dir.join("ployz.sock"))
        .arg("upgrade-worker")
        .arg("--attempt")
        .arg(attempt_id.as_str())
        .kill_on_drop(true);
    let output = timeout(LAUNCH_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            Error::Launch(format!(
                "systemd-run timed out after {}s",
                LAUNCH_TIMEOUT.as_secs()
            ))
        })?
        .map_err(Error::InspectWorker)?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(Error::Launch(if detail.is_empty() {
            format!("systemd-run exited with {}", output.status)
        } else {
            detail
        }))
    }
}

async fn worker_active(attempt_id: MachineUpgradeAttemptId) -> Result<bool, Error> {
    let output = Command::new("systemctl")
        .args(["is-active", "--quiet", &unit_name(attempt_id)])
        .output()
        .await
        .map_err(Error::InspectWorker)?;
    Ok(output.status.success())
}

fn unit_name(attempt_id: MachineUpgradeAttemptId) -> String {
    format!("ployz-upgrade-{attempt_id}.service")
}

fn upgrade_stage(stage: InstallStage) -> MachineUpgradeStage {
    match stage {
        InstallStage::Preparing => MachineUpgradeStage::Preparing,
        InstallStage::Acquiring => MachineUpgradeStage::Acquiring,
        InstallStage::Verifying => MachineUpgradeStage::Verifying,
        InstallStage::Activating => MachineUpgradeStage::Activating,
        InstallStage::Restarting => MachineUpgradeStage::Restarting,
        InstallStage::Readiness => MachineUpgradeStage::Readiness,
    }
}

fn read(data_dir: &Path) -> Result<StoredAttempt, Error> {
    read_optional(data_dir)?.ok_or(Error::NotFound)
}

fn read_optional(data_dir: &Path) -> Result<Option<StoredAttempt>, Error> {
    let bytes = match fs::read(data_dir.join(RECEIPT_FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::Read(error)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(Error::Decode)
}

fn write(data_dir: &Path, stored: &StoredAttempt) -> Result<(), Error> {
    fs::create_dir_all(data_dir).map_err(Error::Write)?;
    let bytes = serde_json::to_vec(stored).map_err(Error::Encode)?;
    crate::filesystem::atomic_write(&data_dir.join(RECEIPT_FILE), &bytes, 0o600)
        .map_err(Error::Write)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTRACT_CASE: &str = "PLOYZ_UPGRADE_CONTRACT_CASE";
    const CONTRACT_ROOT: &str = "PLOYZ_UPGRADE_CONTRACT_ROOT";

    #[test]
    fn stored_attempt_round_trips_without_exposing_local_source_in_response() {
        let attempt_id = MachineUpgradeAttemptId::random();
        let stored = StoredAttempt {
            requested: MachineRelease::parse("beta").unwrap(),
            source: StoredReleaseSource::Local {
                directory: "/root/qualification".into(),
            },
            attempt: MachineUpgradeAttempt::Accepted {
                attempt_id,
                target: MachineVersion::parse("1.2.3-beta.4").unwrap(),
            },
        };
        let encoded = serde_json::to_string(&stored).unwrap();
        assert!(encoded.contains("/root/qualification"));
        assert!(
            !serde_json::to_string(&stored.attempt)
                .unwrap()
                .contains("qualification")
        );
        assert_eq!(
            serde_json::from_str::<StoredAttempt>(&encoded).unwrap(),
            stored
        );
    }

    #[test]
    fn upgrade_attempt_contract() {
        if let Ok(case) = env::var(CONTRACT_CASE) {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(run_contract_case(&case));
            let root = PathBuf::from(env::var_os(CONTRACT_ROOT).unwrap());
            fs::write(root.join("child-completed"), case).unwrap();
            return;
        }

        for case in ["retry-active", "interrupted", "launch-failed"] {
            let root = tempfile::Builder::new()
                .prefix(&format!("ployzd-upgrade-{case}-"))
                .tempdir()
                .unwrap();
            let commands = root.path().join("commands");
            fs::create_dir(&commands).unwrap();
            write_script(
                &commands.join("systemd-run"),
                if case == "launch-failed" {
                    "echo worker launch refused >&2; exit 1"
                } else {
                    "printf '%s\\n' \"$*\" >> \"$PLOYZ_UPGRADE_COMMAND_LOG\""
                },
            );
            write_script(
                &commands.join("systemctl"),
                if case == "interrupted" {
                    "exit 3"
                } else {
                    "exit 0"
                },
            );
            fs::create_dir(root.path().join("release")).unwrap();
            let output = std::process::Command::new(env::current_exe().unwrap())
                .args([
                    "--exact",
                    "installer::upgrade::tests::upgrade_attempt_contract",
                    "--nocapture",
                ])
                .env(CONTRACT_CASE, case)
                .env(CONTRACT_ROOT, root.path())
                .env(QUALIFICATION_RELEASE_DIR, root.path().join("release"))
                .env(
                    "PLOYZ_UPGRADE_COMMAND_LOG",
                    root.path().join("commands.log"),
                )
                .env("PATH", &commands)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{case}: stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                fs::read_to_string(root.path().join("child-completed")).unwrap(),
                case
            );
        }
    }

    async fn run_contract_case(case: &str) {
        let root = PathBuf::from(env::var_os(CONTRACT_ROOT).unwrap());
        let data = root.join("data");
        let run = root.join("run");
        let admission = admission::Admission::new(&run, &data);
        let attempt_id = MachineUpgradeAttemptId::parse("a".repeat(32)).unwrap();
        let request = RequestMachineUpgradeRequest {
            attempt_id,
            release: MachineRelease::parse("1.2.3").unwrap(),
        };
        let guard = admission.try_install().unwrap();
        let accepted = request_locked(request.clone(), &data, &run, &guard)
            .await
            .unwrap();
        drop(guard);

        match case {
            "retry-active" => {
                assert!(matches!(accepted, MachineUpgradeAttempt::Accepted { .. }));
                assert_eq!(
                    existing_request(&request, &data).unwrap(),
                    Some(accepted.clone())
                );
                assert!(matches!(
                    admission.try_mutation(),
                    Err(admission::Error::Busy)
                ));
                let conflict = RequestMachineUpgradeRequest {
                    attempt_id,
                    release: MachineRelease::parse("1.2.4").unwrap(),
                };
                assert!(matches!(
                    existing_request(&conflict, &data),
                    Err(Error::AttemptConflict(id)) if id == attempt_id
                ));

                let guard = admission.try_install().unwrap();
                assert_eq!(
                    request_locked(request, &data, &run, &guard).await.unwrap(),
                    accepted
                );
                let other = RequestMachineUpgradeRequest {
                    attempt_id: MachineUpgradeAttemptId::parse("b".repeat(32)).unwrap(),
                    release: MachineRelease::parse("1.2.3").unwrap(),
                };
                assert!(matches!(
                    request_locked(other, &data, &run, &guard).await,
                    Err(Error::Busy)
                ));
                drop(guard);
                let log = fs::read_to_string(root.join("commands.log")).unwrap();
                assert_eq!(log.lines().count(), 1, "{log}");
                for required in [
                    "--property=Type=exec",
                    "--property=RuntimeMaxSec=15min",
                    "--property=NoNewPrivileges=yes",
                    "--property=ProtectSystem=full",
                    "upgrade-worker",
                    "--attempt",
                    attempt_id.as_str(),
                ] {
                    assert!(log.contains(required), "missing {required}: {log}");
                }
            }
            "interrupted" => {
                let observed = inspect(Some(attempt_id), &data, &run).await.unwrap();
                assert!(matches!(
                    observed,
                    MachineUpgradeAttempt::Interrupted {
                        stage: MachineUpgradeStage::Launching,
                        ..
                    }
                ));
                assert!(!admission.active());
                assert!(admission.try_mutation().is_ok());
            }
            "launch-failed" => {
                assert!(matches!(
                    accepted,
                    MachineUpgradeAttempt::Failed {
                        stage: MachineUpgradeStage::Launching,
                        ref error,
                        ..
                    } if error == "launch Machine upgrade worker: worker launch refused"
                ));
                assert!(!admission.active());
                assert_eq!(
                    inspect(Some(attempt_id), &data, &run).await.unwrap(),
                    accepted
                );
            }
            other => panic!("unknown upgrade contract case {other}"),
        }
    }

    fn write_script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        fs::write(path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
