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

pub use self::release::{ReleaseRequest, ReleaseSource};

const PLOYZ_USER: &str = "ployz";

/// Explicit host work associated with one Machine installation attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Preparation {
    /// Replace the daemon only after confirming the Machine is already prepared.
    SoftwareOnly,
    /// Prepare the host and optionally configure its requested storage.
    PrepareHost {
        /// Explicit storage preparation for a fresh Machine.
        storage: StorageChoice,
        /// Existing operator to add to the Ployz service group during host preparation.
        group_user: Option<String>,
    },
}

/// Choices for one local Machine installation attempt.
#[derive(Clone, Debug)]
pub struct InstallRequest {
    /// A fixed version or a channel resolved once before host mutation.
    pub release: ReleaseRequest,
    /// Trusted release source. Published releases never accept caller-provided URLs.
    pub source: ReleaseSource,
    /// Explicit host preparation or ordinary software-only replacement.
    pub preparation: Preparation,
    /// Write the release and units without starting systemd.
    pub install_only: bool,
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

    #[cfg(test)]
    fn at(root: &Path) -> Self {
        Self {
            bin_dir: root.join("bin"),
            systemd_dir: root.join("systemd"),
            data_dir: root.join("data"),
            run_dir: root.join("run"),
            docker_config: root.join("docker/daemon.json"),
            modprobe_dir: root.join("modprobe"),
        }
    }
}

struct InstallLock(File);

impl Drop for InstallLock {
    fn drop(&mut self) {
        // Forked children can inherit this file descriptor before exec; unlock the shared lock
        // explicitly instead of waiting for every inherited descriptor to close.
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
    require_root()?;
    let _lock = claim_lock(&paths.run_dir)?;
    verify_system(request.install_only)?;
    let target = resolve_release(&request.release, &request.source).await?;

    match &request.preparation {
        Preparation::SoftwareOnly => verify_software_prerequisites(&paths)?,
        Preparation::PrepareHost {
            storage,
            group_user,
        } => {
            prepare_storage(*storage, &paths)?;
            install_prerequisites()?;
            let inherited_group = sudo_user();
            create_user_and_directories(
                group_user.as_deref().or(inherited_group.as_deref()),
                &paths,
            )?;
        }
    }

    let mut restart_required = !paths.systemd_dir.join("ployz.service").is_file();
    restart_required |= install_binaries(&request.source, &paths, &target).await?;
    install_systemd(&paths, request.install_only)?;
    if matches!(request.preparation, Preparation::PrepareHost { .. }) {
        install_docker(&paths, request.install_only).await?;
    }

    let readiness = if request.install_only {
        Readiness::InstallationOnly
    } else {
        if restart_required {
            systemctl("restart daemon", ["restart", "ployz.service"])?;
            systemctl(
                "restart volume plugin",
                ["try-restart", "ployz-volume-plugin.service"],
            )?;
        }
        verify_running_daemon(&paths, &target).await?;
        Readiness::Running
    };
    Ok(InstallOutcome {
        target: target.to_string(),
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

pub(super) fn run_apt<const N: usize>(
    stage: &str,
    args: [&str; N],
    working_directory: Option<&Path>,
) -> Result<Output, Error> {
    let mut command = Command::new("apt-get");
    command
        .args(["-o", "DPkg::Lock::Timeout=300"])
        .args(args)
        .env("DEBIAN_FRONTEND", "noninteractive");
    if let Some(working_directory) = working_directory {
        command.current_dir(working_directory);
    }
    run_command(stage, &mut command)
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        ffi::OsString,
        io::Write,
        os::unix::fs::{PermissionsExt, symlink},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use semver::Version;

    use super::*;

    #[tokio::test]
    async fn installation_interface_contract() {
        if let Ok(case) = env::var("PLOYZ_INSTALLER_CONTRACT_CASE") {
            run_installation_case(&case).await;
            return;
        }

        for case in [
            "success",
            "missing",
            "bad-checksum",
            "corrupt",
            "rejected-executable",
            "hung-executable",
            "software-prerequisite",
            "busy",
        ] {
            let fixture = fixture(case);
            create_installation_fixture(&fixture, case);
            run_contract_child("installation_interface_contract", &fixture, case);
            fs::remove_dir_all(fixture).unwrap();
        }
    }

    #[test]
    fn zfs_candidate_download_contract() {
        if env::var_os("PLOYZ_ZFS_CANDIDATE_CONTRACT").is_some() {
            super::storage::install_zfs_packages("test-kernel").unwrap();
            return;
        }

        let fixture = fixture("zfs-candidate");
        let commands = fixture.join("commands");
        fs::create_dir_all(&commands).unwrap();
        let marker = fixture.join("installed");
        write_script(
            &commands.join("apt-cache"),
            "[ \"$2\" = linux-main-modules-zfs-test-kernel ]",
        );
        write_script(
            &commands.join("dpkg-query"),
            "if [ -f \"$PLOYZ_ZFS_INSTALLED\" ]; then echo /lib/modules/test-kernel/kernel/zfs.ko; exit 0; fi\nexit 1",
        );
        write_script(
            &commands.join("apt-get"),
            "case \"$*\" in\n  *download*) : > module.deb ;;\n  *install*) : > \"$PLOYZ_ZFS_INSTALLED\" ;;\nesac",
        );
        write_script(
            &commands.join("dpkg-deb"),
            "echo /lib/modules/test-kernel/kernel/zfs.ko",
        );
        run_contract_child_with_environment(
            "zfs_candidate_download_contract",
            &fixture,
            OsString::from("PLOYZ_ZFS_CANDIDATE_CONTRACT"),
            OsString::from("1"),
            [(
                OsString::from("PLOYZ_ZFS_INSTALLED"),
                marker.into_os_string(),
            )],
        );
        fs::remove_dir_all(fixture).unwrap();
    }

    async fn run_installation_case(case: &str) {
        let root = PathBuf::from(env::var_os("PLOYZ_INSTALLER_CONTRACT_ROOT").unwrap());
        let paths = InstallPaths::at(&root);
        fs::create_dir_all(&paths.data_dir).unwrap();
        let existing = if case == "success" {
            None
        } else {
            Some(write_existing_daemon(&paths))
        };
        if case == "software-prerequisite" {
            fs::remove_file(root.join("commands/dockerd")).unwrap();
        }

        let request = InstallRequest {
            release: ReleaseRequest::Exact(Version::parse("1.2.3").unwrap()),
            source: ReleaseSource::Local(root.join("release")),
            preparation: Preparation::SoftwareOnly,
            install_only: true,
        };
        let result = if case == "busy" {
            let _held = claim_lock(&paths.run_dir).unwrap();
            install_at(request, paths.clone()).await
        } else {
            install_at(request, paths.clone()).await
        };

        match case {
            "success" => {
                let outcome = result.unwrap();
                assert_eq!(outcome.target, "1.2.3");
                assert_eq!(outcome.readiness, Readiness::InstallationOnly);
                assert!(paths.bin_dir.join("ployzd").is_file());
                assert!(paths.systemd_dir.join("ployz.service").is_file());
            }
            "busy" => assert!(matches!(result, Err(Error::Busy))),
            "software-prerequisite" => assert!(matches!(result, Err(Error::Command { .. }))),
            "missing" | "bad-checksum" | "corrupt" | "rejected-executable" | "hung-executable" => {
                assert!(result.is_err(), "{case} artifact was accepted");
            }
            other => panic!("unknown contract case {other}"),
        }
        if let Some(existing) = existing {
            assert_eq!(fs::read(paths.bin_dir.join("ployzd")).unwrap(), existing);
        }
    }

    fn create_installation_fixture(root: &Path, case: &str) {
        let commands = root.join("commands");
        let release = root.join("release");
        let payload = root.join("payload");
        fs::create_dir_all(&commands).unwrap();
        fs::create_dir_all(&release).unwrap();
        fs::create_dir_all(&payload).unwrap();
        write_script(&commands.join("id"), "if [ \"$1\" = -u ]; then echo 0; fi");
        write_script(&commands.join("dockerd"), "exit 0");
        symlink("/usr/bin/tar", commands.join("tar")).unwrap();

        let daemon = match case {
            "rejected-executable" => "case \"$1\" in version) echo 9.9.9 ;; esac",
            "hung-executable" => "case \"$1\" in version) sleep 1; echo 1.2.3 ;; esac",
            _ => "case \"$1\" in version) echo 1.2.3 ;; esac",
        };
        write_script(&payload.join("ployzd"), daemon);
        write_script(&payload.join("ployz-uninstall"), "exit 0");
        if case != "missing" {
            let archive = release.join(daemon_archive().unwrap());
            let status = Command::new("tar")
                .args(["-czf"])
                .arg(&archive)
                .args(["-C"])
                .arg(&payload)
                .args(["ployzd", "ployz-uninstall"])
                .status()
                .unwrap();
            assert!(status.success());
            let checksum = String::from_utf8(
                Command::new("sha256sum")
                    .arg(&archive)
                    .output()
                    .unwrap()
                    .stdout,
            )
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_owned();
            fs::write(
                release.join("checksums.txt"),
                format!("{checksum}  {}\n", daemon_archive().unwrap()),
            )
            .unwrap();
            if case == "corrupt" {
                fs::OpenOptions::new()
                    .append(true)
                    .open(archive)
                    .unwrap()
                    .write_all(b"corrupt")
                    .unwrap();
            }
        } else {
            fs::write(
                release.join("checksums.txt"),
                format!("{}  {}\n", "0".repeat(64), daemon_archive().unwrap()),
            )
            .unwrap();
        }
        if case == "bad-checksum" {
            fs::write(release.join("checksums.txt"), "bad checksum\n").unwrap();
        }
    }

    fn write_existing_daemon(paths: &InstallPaths) -> Vec<u8> {
        fs::create_dir_all(&paths.bin_dir).unwrap();
        let existing = b"#!/bin/sh\n[ \"$1\" = version ] && echo 1.2.2\n".to_vec();
        fs::write(paths.bin_dir.join("ployzd"), &existing).unwrap();
        fs::set_permissions(
            paths.bin_dir.join("ployzd"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        existing
    }

    fn run_contract_child(test: &str, root: &Path, case: &str) {
        run_contract_child_with_environment(
            test,
            root,
            OsString::from("PLOYZ_INSTALLER_CONTRACT_CASE"),
            OsString::from(case),
            [],
        );
    }

    fn run_contract_child_with_environment<const N: usize>(
        test: &str,
        root: &Path,
        key: OsString,
        value: OsString,
        extra: [(OsString, OsString); N],
    ) {
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(["--exact", test, "--nocapture"])
            .env("PLOYZ_INSTALLER_CONTRACT_ROOT", root)
            .env("PATH", root.join("commands"))
            .env(key, value);
        for (key, value) in extra {
            command.env(key, value);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "contract child {test} failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    fn write_script(path: &Path, body: &str) {
        fs::write(path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn fixture(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!(
            "ployzd-installer-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        root
    }
}
