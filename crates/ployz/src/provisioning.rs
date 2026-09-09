use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, IsTerminal, Write},
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use clap::ArgMatches;
use ployz_core::StorageChoice;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const RELEASE_REPOSITORY: &str = "https://github.com/getployz/ployz2";
const BOOTSTRAP_VERSION_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum ProvisionError {
    #[error("ssh+go provisioning is not implemented; use system ssh")]
    SshGo,
    #[error("remote machine destination is empty")]
    EmptyDestination,
    #[error("remote machine destination is required")]
    MissingDestination,
    #[error("local ssh client not found; install an ssh client")]
    SshClientMissing(#[source] io::Error),
    #[error("run ssh whoami: {0}")]
    Whoami(#[source] io::Error),
    #[error("ssh whoami failed: {0}")]
    WhoamiFailed(String),
    #[error("ssh whoami returned non-UTF-8 output")]
    WhoamiUtf8,
    #[error("ssh whoami returned an empty user")]
    EmptyUser,
    #[error("check remote sudo: {0}")]
    Sudo(#[source] io::Error),
    #[error("remote user {user} could not authenticate or obtain sudo privileges to install Ployz")]
    SudoRequired { user: String },
    #[error("inspect remote Machine platform: {0}")]
    Platform(#[source] io::Error),
    #[error("remote Machine platform inspection failed: {0}")]
    PlatformFailed(String),
    #[error("remote Machine platform inspection returned non-UTF-8 output")]
    PlatformUtf8,
    #[error("Ployz Machine must be Linux")]
    UnsupportedOs,
    #[error("unsupported Machine architecture: {0}")]
    UnsupportedArchitecture(String),
    #[error("{stage}: {source}")]
    BootstrapIo {
        stage: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("{stage} exited with {status}")]
    BootstrapCommand {
        stage: &'static str,
        status: std::process::ExitStatus,
    },
    #[error("bootstrap verification: {0}")]
    BootstrapVerification(String),
    #[error("bootstrap acquisition: {0}")]
    BootstrapDownload(String),
    #[error("transfer bootstrap daemon: {0}")]
    Transfer(#[source] io::Error),
    #[error("bootstrap daemon transfer exited with {status}")]
    TransferFailed { status: std::process::ExitStatus },
    #[error("run Ployz installer: {0}")]
    Install(#[source] io::Error),
    #[error("Ployz installer exited with {status}")]
    InstallFailed { status: std::process::ExitStatus },
    #[error("run this command with sudo")]
    NotRoot,
    #[error("read storage choice: {0}")]
    StorageInput(#[source] io::Error),
    #[error(transparent)]
    StorageChoice(#[from] ployz_core::ValueError),
    #[error("zfs storage preparation requires the installer; remove --no-install")]
    ZfsWithoutInstaller,
}

/// Resolve Machine storage preparation once before provisioning or enrollment.
pub(crate) fn resolve_storage(matches: &ArgMatches) -> Result<StorageChoice, ProvisionError> {
    let storage = match matches.get_one::<StorageChoice>("storage").copied() {
        Some(storage) => storage,
        None if matches.get_flag("yes")
            || !io::stdin().is_terminal()
            || !io::stdout().is_terminal() =>
        {
            StorageChoice::None
        }
        None => {
            print!(
                "Storage preparation [zfs/none] (none keeps this Machine currently stateless): "
            );
            io::stdout().flush().map_err(ProvisionError::StorageInput)?;
            let mut answer = String::new();
            io::stdin()
                .read_line(&mut answer)
                .map_err(ProvisionError::StorageInput)?;
            let answer = answer.trim();
            if answer.is_empty() {
                StorageChoice::None
            } else {
                StorageChoice::parse(answer)?
            }
        }
    };
    if storage == StorageChoice::Zfs && matches.get_flag("no-install") {
        return Err(ProvisionError::ZfsWithoutInstaller);
    }
    announce_storage(storage);
    Ok(storage)
}

pub(crate) fn announce_storage(storage: StorageChoice) {
    if storage == StorageChoice::None {
        println!("Storage: none — this Machine currently supports stateless workloads only.");
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn daemon_archive(architecture: &str) -> Result<&'static str, ProvisionError> {
    match architecture {
        "x86_64" => Ok("ployzd_linux_amd64.tar.gz"),
        "aarch64" => Ok("ployzd_linux_arm64.tar.gz"),
        architecture => Err(ProvisionError::UnsupportedArchitecture(
            architecture.to_owned(),
        )),
    }
}

struct Bootstrap {
    directory: PathBuf,
    daemon: PathBuf,
}

impl Bootstrap {
    fn new() -> Result<Self, ProvisionError> {
        let directory = env::temp_dir().join(format!("ployz-bootstrap-{}", Uuid::new_v4()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|source| ProvisionError::BootstrapIo {
                stage: "create bootstrap directory",
                source,
            })?;
        let daemon = directory.join("ployzd");
        Ok(Self { directory, daemon })
    }
}

impl Drop for Bootstrap {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn bootstrap_source() -> Option<PathBuf> {
    env::var_os("PLOYZ_RELEASE_DIR").map(PathBuf::from)
}

fn acquire_bootstrap(
    architecture: &str,
    source: Option<&Path>,
) -> Result<Bootstrap, ProvisionError> {
    let archive = daemon_archive(architecture)?;
    let bootstrap = Bootstrap::new()?;
    let archive_path = bootstrap.directory.join(archive);
    let checksums_path = bootstrap.directory.join("checksums.txt");
    if let Some(source) = source {
        copy_bootstrap_file(
            &source.join(archive),
            &archive_path,
            "copy bootstrap daemon archive",
        )?;
        copy_bootstrap_file(
            &source.join("checksums.txt"),
            &checksums_path,
            "copy bootstrap release checksums",
        )?;
    } else {
        let repository = env::var("PLOYZ_GITHUB_URL").unwrap_or_else(|_| RELEASE_REPOSITORY.into());
        let base = format!(
            "{repository}/releases/download/v{}",
            env!("CARGO_PKG_VERSION")
        );
        download_bootstrap(
            &format!("{base}/{archive}"),
            &archive_path,
            "download bootstrap daemon archive",
        )?;
        download_bootstrap(
            &format!("{base}/checksums.txt"),
            &checksums_path,
            "download bootstrap release checksums",
        )?;
    }
    verify_bootstrap_checksum(&archive_path, &checksums_path, archive)?;
    let mut extract = Command::new("tar");
    extract
        .arg("-xzf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&bootstrap.directory)
        .arg("ployzd");
    bootstrap_status("extract bootstrap daemon", extract.status())?;
    fs::set_permissions(&bootstrap.daemon, fs::Permissions::from_mode(0o700)).map_err(
        |source| ProvisionError::BootstrapIo {
            stage: "mark bootstrap daemon executable",
            source,
        },
    )?;
    Ok(bootstrap)
}

fn copy_bootstrap_file(
    source: &Path,
    destination: &Path,
    stage: &'static str,
) -> Result<(), ProvisionError> {
    fs::copy(source, destination)
        .map(|_| ())
        .map_err(|source| ProvisionError::BootstrapIo { stage, source })
}

fn download_bootstrap(
    url: &str,
    destination: &Path,
    stage: &'static str,
) -> Result<(), ProvisionError> {
    let url = url.to_owned();
    let bytes = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("{stage}: build download runtime: {error}"))?;
        runtime.block_on(async {
            let client = reqwest::Client::builder()
                .https_only(true)
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(120))
                .user_agent("ployz-bootstrap")
                .build()
                .map_err(|error| format!("{stage}: build download client: {error}"))?;
            client
                .get(url)
                .send()
                .await
                .map_err(|error| format!("{stage}: {error}"))?
                .error_for_status()
                .map_err(|error| format!("{stage}: {error}"))?
                .bytes()
                .await
                .map(|bytes| bytes.to_vec())
                .map_err(|error| format!("{stage}: {error}"))
        })
    })
    .join()
    .map_err(|_| ProvisionError::BootstrapDownload(format!("{stage}: worker panicked")))?
    .map_err(ProvisionError::BootstrapDownload)?;
    fs::write(destination, bytes).map_err(|source| ProvisionError::BootstrapIo { stage, source })
}

fn bootstrap_status(
    stage: &'static str,
    result: io::Result<std::process::ExitStatus>,
) -> Result<(), ProvisionError> {
    let status = result.map_err(|source| ProvisionError::BootstrapIo { stage, source })?;
    if status.success() {
        Ok(())
    } else {
        Err(ProvisionError::BootstrapCommand { stage, status })
    }
}

fn verify_bootstrap_checksum(
    archive: &Path,
    checksums: &Path,
    archive_name: &str,
) -> Result<(), ProvisionError> {
    let checksums =
        fs::read_to_string(checksums).map_err(|source| ProvisionError::BootstrapIo {
            stage: "read bootstrap release checksums",
            source,
        })?;
    let expected = checksums
        .lines()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            let hash = fields.next()?;
            let name = fields.next()?.trim_start_matches('*');
            (name == archive_name
                && hash.len() == 64
                && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
        })
        .ok_or_else(|| {
            ProvisionError::BootstrapVerification(format!(
                "checksums.txt has no SHA-256 hash for {archive_name}"
            ))
        })?;
    let bytes = fs::read(archive).map_err(|source| ProvisionError::BootstrapIo {
        stage: "read bootstrap daemon archive",
        source,
    })?;
    let actual = hex::encode(Sha256::digest(bytes));
    if actual == expected {
        Ok(())
    } else {
        Err(ProvisionError::BootstrapVerification(format!(
            "{archive_name} checksum was {actual}, expected {expected}"
        )))
    }
}

fn command_output_with_timeout(command: &mut Command, timeout: Duration) -> io::Result<Output> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.kill()?;
    let _ = child.wait();
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("timed out after {} seconds", timeout.as_secs()),
    ))
}

fn verify_bootstrap_version(command: &mut Command) -> Result<(), ProvisionError> {
    let output =
        command_output_with_timeout(command, BOOTSTRAP_VERSION_TIMEOUT).map_err(|source| {
            ProvisionError::BootstrapIo {
                stage: "preflight bootstrap daemon",
                source,
            }
        })?;
    if !output.status.success() {
        return Err(ProvisionError::BootstrapCommand {
            stage: "preflight bootstrap daemon",
            status: output.status,
        });
    }
    let observed = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if observed == env!("CARGO_PKG_VERSION") {
        Ok(())
    } else {
        Err(ProvisionError::BootstrapVerification(format!(
            "bootstrap daemon reported version {observed:?}, expected {}",
            env!("CARGO_PKG_VERSION")
        )))
    }
}

enum Preparation<'user> {
    Host {
        storage: StorageChoice,
        group_user: Option<&'user str>,
    },
    SoftwareOnly,
}

fn normalized_version(version: &str) -> &str {
    let version = version.strip_prefix('v').unwrap_or(version);
    match version {
        "" | "latest" => "stable",
        version => version,
    }
}

fn install_arguments(
    version: &str,
    preparation: Preparation<'_>,
    release_dir: Option<&Path>,
) -> Vec<OsString> {
    let mut arguments = vec![
        "install".into(),
        "--version".into(),
        normalized_version(version).into(),
    ];
    match preparation {
        Preparation::Host {
            storage,
            group_user,
        } => {
            arguments.extend(["--storage".into(), storage.as_str().into()]);
            if let Some(user) = group_user {
                arguments.extend(["--group-user".into(), user.into()]);
            }
        }
        Preparation::SoftwareOnly => arguments.push("--software-only".into()),
    }
    if let Some(directory) = release_dir {
        arguments.extend(["--release-dir".into(), directory.as_os_str().to_owned()]);
    }
    arguments
}

fn installer_status(result: io::Result<std::process::ExitStatus>) -> Result<(), ProvisionError> {
    let status = result.map_err(ProvisionError::Install)?;
    if status.success() {
        Ok(())
    } else {
        Err(ProvisionError::InstallFailed { status })
    }
}

fn ssh_parts(destination: &str) -> Result<(String, Option<String>), ProvisionError> {
    if destination.starts_with("ssh+go://") {
        return Err(ProvisionError::SshGo);
    }
    let destination = destination
        .strip_prefix("ssh://")
        .or_else(|| destination.strip_prefix("ssh+cli://"))
        .unwrap_or(destination);
    if destination.is_empty() {
        return Err(ProvisionError::EmptyDestination);
    }
    if let Some((host, port)) = destination.rsplit_once(':')
        && !port.is_empty()
        && port.chars().all(|character| character.is_ascii_digit())
    {
        return Ok((host.to_owned(), Some(port.to_owned())));
    }
    Ok((destination.to_owned(), None))
}

fn ssh_key(matches: &ArgMatches) -> PathBuf {
    let key = matches
        .get_one::<String>("ssh-key")
        .expect("ssh-key has a default");
    key.strip_prefix("~/").map_or_else(
        || PathBuf::from(key),
        |relative| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(relative)
        },
    )
}

fn ssh_command(matches: &ArgMatches) -> Result<(Command, String), ProvisionError> {
    let destination = matches
        .get_one::<String>("destination")
        .ok_or(ProvisionError::MissingDestination)?;
    let (destination, port) = ssh_parts(destination)?;
    let mut command = Command::new("ssh");
    command.arg("-o").arg(format!(
        "ConnectTimeout={}",
        crate::cli::ssh_timeout(matches).as_secs()
    ));
    // Provisioning may wait for human authentication; only network setup is timed.
    command.args(["-o", "BatchMode=no", "-tt"]);
    command.args(crate::connect::ssh_control_args(
        crate::connect::control_path().as_deref(),
    ));
    command.stdin(Stdio::inherit());
    command.arg("-i").arg(ssh_key(matches));
    if let Some(port) = port {
        command.arg("-p").arg(port);
    }
    Ok((command, destination))
}

fn scp_command(matches: &ArgMatches) -> Result<(Command, String), ProvisionError> {
    let destination = matches
        .get_one::<String>("destination")
        .ok_or(ProvisionError::MissingDestination)?;
    let (destination, port) = ssh_parts(destination)?;
    let mut command = Command::new("scp");
    command.arg("-o").arg(format!(
        "ConnectTimeout={}",
        crate::cli::ssh_timeout(matches).as_secs()
    ));
    command.args(["-o", "BatchMode=no"]);
    command.args(crate::connect::ssh_control_args(
        crate::connect::control_path().as_deref(),
    ));
    command.arg("-i").arg(ssh_key(matches));
    if let Some(port) = port {
        command.arg("-P").arg(port);
    }
    Ok((command, destination))
}

fn scp_destination(destination: &str, path: &str) -> String {
    let (user, host) = destination
        .split_once('@')
        .expect("validated SSH destinations include a user");
    if host.contains(':') && !host.starts_with('[') {
        format!("{user}@[{host}]:{path}")
    } else {
        format!("{destination}:{path}")
    }
}

fn remote_platform(matches: &ArgMatches) -> Result<String, ProvisionError> {
    let (mut platform, destination) = ssh_command(matches)?;
    let output = platform
        .arg(destination)
        .arg("uname -s; uname -m")
        .output()
        .map_err(ProvisionError::Platform)?;
    if !output.status.success() {
        return Err(ProvisionError::PlatformFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let output = String::from_utf8(output.stdout).map_err(|_| ProvisionError::PlatformUtf8)?;
    let mut lines = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    if lines.next() != Some("Linux") {
        return Err(ProvisionError::UnsupportedOs);
    }
    lines
        .next()
        .map(str::to_owned)
        .ok_or_else(|| ProvisionError::PlatformFailed("uname returned no architecture".into()))
}

fn remote_command(program: &str, arguments: &[OsString], via_sudo: bool) -> String {
    let mut command = String::new();
    if via_sudo {
        command.push_str("sudo ");
    }
    command.push_str(&shell_quote(program));
    for argument in arguments {
        command.push(' ');
        command.push_str(&shell_quote(&argument.to_string_lossy()));
    }
    command
}

fn cleanup_remote_bootstrap(matches: &ArgMatches, path: &str) {
    if let Ok((mut cleanup, destination)) = ssh_command(matches) {
        let _ = cleanup
            .arg(destination)
            .arg(format!("rm -f -- {}", shell_quote(path)))
            .status();
    }
}

pub fn provision(matches: &ArgMatches, storage: StorageChoice) -> Result<(), ProvisionError> {
    let (mut whoami, destination) = ssh_command(matches)?;
    let output = whoami
        .arg(&destination)
        .arg("whoami")
        .output()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                ProvisionError::SshClientMissing(error)
            } else {
                ProvisionError::Whoami(error)
            }
        })?;
    if !output.status.success() {
        return Err(ProvisionError::WhoamiFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let user = String::from_utf8(output.stdout).map_err(|_| ProvisionError::WhoamiUtf8)?;
    let user = user.trim();
    if user.is_empty() {
        return Err(ProvisionError::EmptyUser);
    }

    if user != "root" {
        let (mut sudo, destination) = ssh_command(matches)?;
        let status = sudo
            .arg(destination)
            .arg("sudo true")
            .status()
            .map_err(ProvisionError::Sudo)?;
        if !status.success() {
            return Err(ProvisionError::SudoRequired {
                user: user.to_owned(),
            });
        }
    }

    let architecture = remote_platform(matches)?;
    let bootstrap = acquire_bootstrap(&architecture, bootstrap_source().as_deref())?;
    let remote_bootstrap = format!("/tmp/ployz-bootstrap-{}", Uuid::new_v4());
    let (mut transfer, destination) = scp_command(matches)?;
    let status = transfer
        .arg(&bootstrap.daemon)
        .arg(scp_destination(&destination, &remote_bootstrap))
        .status()
        .map_err(ProvisionError::Transfer)?;
    if !status.success() {
        cleanup_remote_bootstrap(matches, &remote_bootstrap);
        return Err(ProvisionError::TransferFailed { status });
    }

    let preflight = (|| {
        let (mut verify, destination) = ssh_command(matches)?;
        let command = format!(
            "chmod 700 {path} && {path} version",
            path = shell_quote(&remote_bootstrap)
        );
        let output = verify
            .arg(destination)
            .arg(command)
            .output()
            .map_err(|source| ProvisionError::BootstrapIo {
                stage: "preflight remote bootstrap daemon",
                source,
            })?;
        if !output.status.success() {
            return Err(ProvisionError::BootstrapCommand {
                stage: "preflight remote bootstrap daemon",
                status: output.status,
            });
        }
        let observed = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if observed != env!("CARGO_PKG_VERSION") {
            return Err(ProvisionError::BootstrapVerification(format!(
                "remote bootstrap daemon reported version {observed:?}, expected {}",
                env!("CARGO_PKG_VERSION")
            )));
        }
        Ok(())
    })();
    if let Err(error) = preflight {
        cleanup_remote_bootstrap(matches, &remote_bootstrap);
        return Err(error);
    }

    let version = matches
        .get_one::<String>("version")
        .expect("version has a default");
    let via_sudo = user != "root";
    let arguments = install_arguments(
        version,
        Preparation::Host {
            storage,
            group_user: via_sudo.then_some(user),
        },
        None,
    );
    let remote = remote_command(&remote_bootstrap, &arguments, via_sudo);
    let (mut install, destination) = ssh_command(matches)?;
    let result = installer_status(install.arg(destination).arg(remote).status());
    cleanup_remote_bootstrap(matches, &remote_bootstrap);
    result
}

/// Install and start local `ployzd` through a verified temporary daemon.
///
/// # Errors
///
/// Returns [`ProvisionError::NotRoot`] when this process is not root.
/// Returns [`ProvisionError::Install`] when the installer cannot be spawned,
/// or [`ProvisionError::InstallFailed`] when it exits non-zero.
pub fn provision_local(version: &str, storage: StorageChoice) -> Result<(), ProvisionError> {
    let group_user = env::var("SUDO_USER").ok().filter(|user| !user.is_empty());
    provision_local_with(
        version,
        Preparation::Host {
            storage,
            group_user: group_user.as_deref(),
        },
    )
}

/// Synchronize the local daemon binary without preparing host software or storage.
///
/// # Errors
///
/// Returns the first bootstrap acquisition, verification, or installation failure.
pub fn synchronize_local_daemon() -> Result<(), ProvisionError> {
    provision_local_with(env!("CARGO_PKG_VERSION"), Preparation::SoftwareOnly)
}

fn provision_local_with(version: &str, preparation: Preparation<'_>) -> Result<(), ProvisionError> {
    if !process_is_root() {
        return Err(ProvisionError::NotRoot);
    }
    if env::consts::OS != "linux" {
        return Err(ProvisionError::UnsupportedOs);
    }
    let bootstrap = acquire_bootstrap(env::consts::ARCH, bootstrap_source().as_deref())?;
    let mut version_command = Command::new(&bootstrap.daemon);
    version_command.arg("version");
    verify_bootstrap_version(&mut version_command)?;
    let release_dir = (normalized_version(version) == env!("CARGO_PKG_VERSION"))
        .then_some(bootstrap.directory.as_path());
    installer_status(
        Command::new(&bootstrap.daemon)
            .args(install_arguments(version, preparation, release_dir))
            .status(),
    )
}

pub(crate) fn process_is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|uid| uid.trim() == "0")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ployz_core::StorageChoice;

    #[test]
    fn provisioning_ssh_allows_interactive_authentication_with_a_network_timeout() {
        for (extra, seconds) in [(vec![], "5"), (vec!["--ssh-timeout", "17"], "17")] {
            let mut args = vec!["ployz", "machine", "add", "root@host"];
            args.extend(extra);
            let root = crate::cli::command().try_get_matches_from(args).unwrap();
            let matches = root
                .subcommand_matches("machine")
                .unwrap()
                .subcommand_matches("add")
                .unwrap();
            let (command, _) = ssh_command(matches).unwrap();
            let args: Vec<_> = command.get_args().collect();
            assert!(args.contains(&std::ffi::OsStr::new("BatchMode=no")));
            assert!(args.contains(&std::ffi::OsStr::new("-tt")));
            for arg in crate::connect::ssh_control_args(crate::connect::control_path().as_deref()) {
                assert!(args.contains(&std::ffi::OsStr::new(&arg)));
            }
            assert!(
                command
                    .get_args()
                    .any(|arg| arg == format!("ConnectTimeout={seconds}").as_str())
            );
        }
    }

    #[test]
    fn scp_brackets_bare_ipv6_hosts() {
        assert_eq!(
            scp_destination("deploy@2001:db8::1", "/tmp/bootstrap"),
            "deploy@[2001:db8::1]:/tmp/bootstrap"
        );
        assert_eq!(
            scp_destination("deploy@[2001:db8::1]", "/tmp/bootstrap"),
            "deploy@[2001:db8::1]:/tmp/bootstrap"
        );
    }

    #[test]
    fn shared_installer_arguments_keep_bootstrap_and_target_distinct() {
        assert_eq!(
            install_arguments(
                "v1.2.3",
                Preparation::Host {
                    storage: StorageChoice::None,
                    group_user: None,
                },
                None,
            ),
            ["install", "--version", "1.2.3", "--storage", "none"]
                .map(OsString::from)
                .to_vec()
        );
        assert_eq!(normalized_version("latest"), "stable");
        assert_eq!(
            install_arguments(
                "1.2.3",
                Preparation::Host {
                    storage: StorageChoice::Zfs,
                    group_user: Some("deploy"),
                },
                None,
            ),
            [
                "install",
                "--version",
                "1.2.3",
                "--storage",
                "zfs",
                "--group-user",
                "deploy",
            ]
            .map(OsString::from)
            .to_vec()
        );
        assert_eq!(
            install_arguments("1.2.3", Preparation::SoftwareOnly, None),
            ["install", "--version", "1.2.3", "--software-only"]
                .map(OsString::from)
                .to_vec()
        );
    }

    #[test]
    fn storage_resolution_honors_explicit_and_safe_noninteractive_choices() {
        assert_eq!(
            storage_matches(["ployz", "machine", "add", "root@host", "--storage", "zfs"]),
            StorageChoice::Zfs
        );
        assert_eq!(
            storage_matches(["ployz", "machine", "init", "root@host", "--storage", "none"]),
            StorageChoice::None
        );
        assert_eq!(
            storage_matches(["ployz", "machine", "add", "root@host", "--yes"]),
            StorageChoice::None
        );
    }

    #[test]
    fn zfs_requires_the_installer() {
        let matches = crate::cli::command()
            .try_get_matches_from([
                "ployz",
                "machine",
                "add",
                "root@host",
                "--storage",
                "zfs",
                "--no-install",
            ])
            .unwrap();
        assert_eq!(
            resolve_storage(
                matches
                    .subcommand_matches("machine")
                    .unwrap()
                    .subcommand_matches("add")
                    .unwrap(),
            )
            .unwrap_err()
            .to_string(),
            "zfs storage preparation requires the installer; remove --no-install"
        );
    }

    fn storage_matches<const N: usize>(args: [&str; N]) -> StorageChoice {
        let matches = crate::cli::command().try_get_matches_from(args).unwrap();
        let (_, matches) = matches
            .subcommand_matches("machine")
            .unwrap()
            .subcommand()
            .unwrap();
        resolve_storage(matches).unwrap()
    }

    #[test]
    fn local_provision_requires_root() {
        if process_is_root() {
            return;
        }
        assert!(matches!(
            provision_local(env!("CARGO_PKG_VERSION"), StorageChoice::None),
            Err(ProvisionError::NotRoot)
        ));
    }
}
