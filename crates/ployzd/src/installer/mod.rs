//! Bounded local installation of a Ployz Machine release.

mod host;
mod release;
mod storage;

use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output},
};

use fs2::FileExt;
use ployz_core::StorageChoice;
use thiserror::Error;

use self::{
    host::{
        create_user_and_directories, install_docker, install_prerequisites, install_systemd,
        verify_running_daemon, verify_software_prerequisites,
    },
    release::{install_binaries, resolve_release},
    storage::prepare_storage,
};

const PLOYZ_USER: &str = "ployz";

#[derive(Clone, Debug)]
pub struct InstallRequest {
    /// A fixed version or the stable/beta channel to resolve once.
    pub version: String,
    /// Explicit storage preparation for a fresh Machine.
    pub storage: StorageChoice,
    /// Whether host prerequisites may be prepared. `false` is software-only replacement.
    pub prepare_host: bool,
    /// Write the release and units without starting systemd.
    pub install_only: bool,
    /// Existing operator to add to the Ployz service group while preparing the host.
    pub group_user: Option<String>,
    /// Local release directory used by offline qualification.
    pub release_dir: Option<PathBuf>,
}

/// What installation directly observed after activation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Readiness {
    /// Files and unit definitions were installed, but systemd was deliberately not contacted.
    InstallationOnly,
    /// systemd reported the daemon active and its process is the verified target executable.
    Running,
}

/// The bounded outcome of one local installation attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallOutcome {
    /// Exact release selected before host mutation.
    pub target: String,
    /// Whether the daemon executable changed.
    pub replaced: bool,
    /// Readiness observed before the command returned.
    pub readiness: Readiness,
}

/// Installation failures name the first stage that did not complete.
#[derive(Debug, Error)]
pub enum Error {
    #[error("run this command with sudo")]
    NotRoot,
    #[error("another Ployz installation is active")]
    Busy,
    #[error(
        "software-only replacement cannot prepare {storage} storage; run without --software-only"
    )]
    StorageRequiresHostPreparation { storage: StorageChoice },
    #[error("nightly is not a supported release channel")]
    Nightly,
    #[error("invalid Ployz release version '{value}'; expected X.Y.Z or X.Y.Z-beta.N")]
    InvalidVersion { value: String },
    #[error("Ployz Machine must be Linux")]
    UnsupportedOs,
    #[error("unsupported architecture: {0}")]
    UnsupportedArchitecture(String),
    #[error("Ployz requires systemd")]
    SystemdRequired,
    #[error("release selection: {0}")]
    ReleaseSelection(String),
    #[error("artifact verification: {0}")]
    Verification(String),
    #[error("{stage}: {message}")]
    Command { stage: String, message: String },
    #[error("{stage}: {source}")]
    Io {
        stage: &'static str,
        #[source]
        source: io::Error,
    },
}

#[derive(Clone, Debug)]
pub(super) struct InstallPaths {
    pub(super) bin_dir: PathBuf,
    pub(super) systemd_dir: PathBuf,
    pub(super) data_dir: PathBuf,
    pub(super) run_dir: PathBuf,
    pub(super) docker_config: PathBuf,
    pub(super) modprobe_dir: PathBuf,
}

impl InstallPaths {
    fn system() -> Self {
        Self {
            bin_dir: PathBuf::from("/usr/local/bin"),
            systemd_dir: PathBuf::from("/etc/systemd/system"),
            data_dir: PathBuf::from("/var/lib/ployz"),
            run_dir: PathBuf::from("/run/ployz"),
            docker_config: PathBuf::from("/etc/docker/daemon.json"),
            modprobe_dir: PathBuf::from("/etc/modprobe.d"),
        }
    }
}

struct InstallLock(File);

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Install the selected release through the one Machine-local installation seam.
///
/// # Errors
///
/// Returns the observed first failed installation stage. Acquisition and preflight failures
/// happen before activation, so the installed daemon remains untouched.
pub async fn install(request: InstallRequest) -> Result<InstallOutcome, Error> {
    install_at(request, InstallPaths::system()).await
}

async fn install_at(request: InstallRequest, paths: InstallPaths) -> Result<InstallOutcome, Error> {
    if !request.prepare_host && request.storage != StorageChoice::None {
        return Err(Error::StorageRequiresHostPreparation {
            storage: request.storage,
        });
    }
    require_root()?;
    let _lock = claim_lock(&paths.run_dir)?;
    verify_system(request.install_only)?;
    let target = resolve_release(&request).await?;

    if request.prepare_host {
        prepare_storage(request.storage, &paths)?;
        install_prerequisites()?;
        let inherited_group = sudo_user();
        create_user_and_directories(
            request.group_user.as_deref().or(inherited_group.as_deref()),
            &paths,
        )?;
    } else {
        verify_software_prerequisites(&paths)?;
    }

    let mut replaced = !paths.systemd_dir.join("ployz.service").is_file();
    replaced |= install_binaries(&request, &paths, &target).await?;
    install_systemd(&paths, request.install_only)?;
    if request.prepare_host {
        install_docker(&paths, request.install_only).await?;
    }

    let readiness = if request.install_only {
        Readiness::InstallationOnly
    } else {
        if replaced {
            systemctl("restart daemon", ["restart", "ployz.service"])?;
            systemctl(
                "restart volume plugin",
                ["try-restart", "ployz-volume-plugin.service"],
            )?;
        }
        verify_running_daemon(&paths, &target)?;
        Readiness::Running
    };
    Ok(InstallOutcome {
        target: target.as_string(),
        replaced,
        readiness,
    })
}

fn require_root() -> Result<(), Error> {
    let mut command = Command::new("id");
    command.arg("-u");
    let output = run_command("check installation privilege", &mut command)?;
    if String::from_utf8_lossy(&output.stdout).trim() == "0" {
        Ok(())
    } else {
        Err(Error::NotRoot)
    }
}

fn claim_lock(run_dir: &Path) -> Result<InstallLock, Error> {
    fs::create_dir_all(run_dir).map_err(|source| Error::Io {
        stage: "create installation lock directory",
        source,
    })?;
    fs::set_permissions(run_dir, fs::Permissions::from_mode(0o750)).map_err(|source| {
        Error::Io {
            stage: "set installation lock directory permissions",
            source,
        }
    })?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(run_dir.join(".install.lock"))
        .map_err(|source| Error::Io {
            stage: "open installation lock",
            source,
        })?;
    file.try_lock_exclusive().map_err(|source| {
        if source.kind() == io::ErrorKind::WouldBlock {
            Error::Busy
        } else {
            Error::Io {
                stage: "lock installation",
                source,
            }
        }
    })?;
    Ok(InstallLock(file))
}

fn verify_system(install_only: bool) -> Result<(), Error> {
    if std::env::consts::OS != "linux" {
        return Err(Error::UnsupportedOs);
    }
    daemon_archive()?;
    if !install_only && !Path::new("/run/systemd/system").is_dir() {
        return Err(Error::SystemdRequired);
    }
    Ok(())
}

pub(super) fn daemon_archive() -> Result<&'static str, Error> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("ployzd_linux_amd64.tar.gz"),
        "aarch64" => Ok("ployzd_linux_arm64.tar.gz"),
        architecture => Err(Error::UnsupportedArchitecture(architecture.into())),
    }
}

fn sudo_user() -> Option<String> {
    std::env::var("SUDO_USER")
        .ok()
        .filter(|user| !user.is_empty())
}

pub(super) fn command_exists(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| {
            let candidate = directory.join(name);
            candidate.is_file()
                && candidate
                    .metadata()
                    .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
        })
    })
}

pub(super) fn run_command(
    stage: impl Into<String>,
    command: &mut Command,
) -> Result<Output, Error> {
    let stage = stage.into();
    let output = command.output().map_err(|source| Error::Io {
        stage: "start host command",
        source,
    })?;
    if output.status.success() {
        Ok(output)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(Error::Command {
            stage,
            message: if stderr.is_empty() {
                format!("exited with {}", output.status)
            } else {
                stderr
            },
        })
    }
}

pub(super) fn systemctl<const N: usize>(stage: &str, args: [&str; N]) -> Result<Output, Error> {
    let mut command = Command::new("systemctl");
    command.args(args);
    run_command(stage, &mut command)
}

pub(super) fn run_host<const N: usize>(
    stage: &str,
    program: &str,
    args: [&str; N],
) -> Result<Output, Error> {
    let mut command = Command::new(program);
    command.args(args);
    run_command(stage, &mut command)
}

pub(super) fn run_apt<const N: usize>(stage: &str, args: [&str; N]) -> Result<Output, Error> {
    let mut command = Command::new("apt-get");
    command.args(["-o", "DPkg::Lock::Timeout=300"]);
    command.args(args);
    run_command(stage, &mut command)
}

pub(super) fn run_apt_with_env<const N: usize>(
    stage: &str,
    args: [&str; N],
) -> Result<Output, Error> {
    let mut command = Command::new("apt-get");
    command
        .args(["-o", "DPkg::Lock::Timeout=300"])
        .args(args)
        .env("DEBIAN_FRONTEND", "noninteractive");
    run_command(stage, &mut command)
}

#[cfg(test)]
mod tests {
    use super::release::{Release, checksum_for};
    use super::*;

    #[test]
    fn release_versions_accept_published_shapes_and_order_beta_before_stable() {
        assert_eq!(Release::parse("v1.2.3").unwrap().as_string(), "1.2.3");
        assert_eq!(
            Release::parse("1.2.3-beta.4").unwrap().as_string(),
            "1.2.3-beta.4"
        );
        assert!(Release::parse("1.2.3-beta.4").unwrap() < Release::parse("1.2.3").unwrap());
    }

    #[test]
    fn release_versions_reject_non_published_shapes() {
        for invalid in ["", "v", "1.2", "1.2.3-rc.1", "1.2.3-beta.x", "1.2.3.4"] {
            assert!(Release::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn checksum_parser_requires_the_requested_daemon_archive() {
        let archive = "ployzd_linux_amd64.tar.gz";
        let checksum = "2d20468cbeb9745b56fdf16897f963773baeecf45c4ea2e00a46e8d82da6ee9f";
        assert_eq!(
            checksum_for(format!("{checksum}  {archive}\n").as_bytes(), archive),
            Some(checksum.into())
        );
        assert_eq!(checksum_for(b"bad  other.tar.gz\n", archive), None);
    }

    #[tokio::test]
    async fn software_only_rejects_storage_preparation_before_host_mutation() {
        let error = install_at(
            InstallRequest {
                version: "1.2.3".into(),
                storage: StorageChoice::Zfs,
                prepare_host: false,
                install_only: true,
                group_user: None,
                release_dir: None,
            },
            InstallPaths::system(),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            Error::StorageRequiresHostPreparation { .. }
        ));
    }
}
