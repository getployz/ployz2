//! Release selection, acquisition, verification, and activation.

use std::{
    cmp::Ordering as Compare,
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use sha2::{Digest, Sha256};

use super::{Error, InstallPaths, InstallRequest, daemon_archive, run_command};

const RELEASE_REPOSITORY: &str = "https://github.com/getployz/ployz2";
const CHANNEL_URL: &str = "https://ployz.sh";
const RELEASE_API: &str = "https://api.github.com/repos/getployz/ployz2";
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Release {
    major: u64,
    minor: u64,
    patch: u64,
    /// `None` is a stable release, which sorts after its beta releases.
    beta: Option<u64>,
}

impl Release {
    pub(super) fn parse(value: &str) -> Result<Self, Error> {
        let value = value.strip_prefix('v').unwrap_or(value);
        let (core, beta) = match value.split_once("-beta.") {
            Some((core, beta)) => (core, Some(beta)),
            None => (value, None),
        };
        if core.contains('-') {
            return Err(Error::InvalidVersion {
                value: value.into(),
            });
        }
        let mut pieces = core.split('.');
        let parse_part = |part: Option<&str>| {
            part.filter(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
                .and_then(|part| part.parse().ok())
        };
        let (Some(major), Some(minor), Some(patch), None) = (
            parse_part(pieces.next()),
            parse_part(pieces.next()),
            parse_part(pieces.next()),
            pieces.next(),
        ) else {
            return Err(Error::InvalidVersion {
                value: value.into(),
            });
        };
        let beta = match beta {
            Some(beta) if beta.bytes().all(|byte| byte.is_ascii_digit()) => {
                beta.parse().map(Some).map_err(|_| Error::InvalidVersion {
                    value: value.into(),
                })?
            }
            Some(_) => {
                return Err(Error::InvalidVersion {
                    value: value.into(),
                });
            }
            None => None,
        };
        Ok(Self {
            major,
            minor,
            patch,
            beta,
        })
    }

    pub(super) fn as_string(&self) -> String {
        let Self {
            major,
            minor,
            patch,
            beta,
        } = self;
        match beta {
            Some(beta) => format!("{major}.{minor}.{patch}-beta.{beta}"),
            None => format!("{major}.{minor}.{patch}"),
        }
    }
}

impl Ord for Release {
    fn cmp(&self, other: &Self) -> Compare {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.beta, other.beta) {
                (Some(left), Some(right)) => left.cmp(&right),
                (Some(_), None) => Compare::Less,
                (None, Some(_)) => Compare::Greater,
                (None, None) => Compare::Equal,
            })
    }
}

impl PartialOrd for Release {
    fn partial_cmp(&self, other: &Self) -> Option<Compare> {
        Some(self.cmp(other))
    }
}

pub(super) async fn resolve_release(request: &InstallRequest) -> Result<Release, Error> {
    let requested = request
        .version
        .strip_prefix('v')
        .unwrap_or(&request.version);
    if requested == "nightly" {
        return Err(Error::Nightly);
    }
    match requested {
        "" | "latest" | "stable" => {
            let value = release_file(request, "stable").await?;
            Release::parse(value.trim())
        }
        "beta" => {
            let value = release_file(request, "beta").await?;
            Release::parse(value.trim())
        }
        version => Release::parse(version),
    }
}

async fn release_file(request: &InstallRequest, name: &str) -> Result<String, Error> {
    let bytes = if let Some(directory) = &request.release_dir {
        fs::read(directory.join(name)).map_err(|source| Error::Io {
            stage: "read local release channel",
            source,
        })?
    } else {
        fetch(&format!("{CHANNEL_URL}/{name}"), "resolve release channel").await?
    };
    String::from_utf8(bytes)
        .map_err(|error| Error::ReleaseSelection(format!("{name} channel is not UTF-8: {error}")))
}

pub(super) async fn fetch(url: &str, stage: &str) -> Result<Vec<u8>, Error> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .user_agent("ployzd-installer")
        .build()
        .map_err(|error| Error::ReleaseSelection(format!("build download client: {error}")))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| Error::ReleaseSelection(format!("{stage}: {error}")))?
        .error_for_status()
        .map_err(|error| Error::ReleaseSelection(format!("{stage}: {error}")))?;
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| Error::ReleaseSelection(format!("{stage}: {error}")))
}

fn release_url(target: &Release, file: &str) -> String {
    format!(
        "{RELEASE_REPOSITORY}/releases/download/v{}/{file}",
        target.as_string()
    )
}

pub(super) async fn install_binaries(
    request: &InstallRequest,
    paths: &InstallPaths,
    target: &Release,
) -> Result<bool, Error> {
    let installed = installed_release(&paths.bin_dir.join("ployzd"));
    let pinned = !matches!(
        request
            .version
            .strip_prefix('v')
            .unwrap_or(&request.version),
        "" | "latest" | "stable" | "beta"
    );
    let replace = match installed {
        None => true,
        Some(ref installed) if request.release_dir.is_some() => true,
        Some(ref installed) if pinned => installed != target,
        Some(ref installed) => installed < target,
    };
    if !replace {
        println!(
            "ployzd {} retained",
            installed
                .expect("replace covers missing release")
                .as_string()
        );
        return Ok(false);
    }

    let archive = daemon_archive()?;
    let stage = Staging::new(&paths.bin_dir)?;
    let archive_path = stage.path.join(archive);
    let checksum = published_checksum(request, target, archive).await?;
    let archive_bytes = if let Some(directory) = &request.release_dir {
        fs::read(directory.join(archive)).map_err(|source| Error::Io {
            stage: "read local daemon archive",
            source,
        })?
    } else {
        fetch(&release_url(target, archive), "download daemon archive").await?
    };
    write_private(&archive_path, &archive_bytes, "stage daemon archive")?;
    verify_checksum(&archive_path, &checksum)?;
    extract_archive(&archive_path, &stage.path)?;
    let daemon = stage.path.join("ployzd");
    let uninstall = stage.path.join("ployz-uninstall");
    verify_executable(&daemon, target)?;
    verify_uninstall(&uninstall)?;
    activate(&daemon, &uninstall, paths)?;
    Ok(true)
}

pub(super) fn installed_release(path: &Path) -> Option<Release> {
    if !path.is_file() {
        return None;
    }
    let output = Command::new(path).arg("version").output().ok()?;
    output.status.success().then_some(())?;
    Release::parse(String::from_utf8_lossy(&output.stdout).trim()).ok()
}

async fn published_checksum(
    request: &InstallRequest,
    target: &Release,
    archive: &str,
) -> Result<String, Error> {
    let checksums = if let Some(directory) = &request.release_dir {
        fs::read(directory.join("checksums.txt")).map_err(|source| Error::Io {
            stage: "read local release checksums",
            source,
        })?
    } else {
        fetch(
            &release_url(target, "checksums.txt"),
            "download release checksums",
        )
        .await?
    };
    if let Some(checksum) = checksum_for(&checksums, archive) {
        return Ok(checksum);
    }
    if request.release_dir.is_some() {
        return Err(Error::Verification(format!(
            "checksums.txt has no hash for {archive}"
        )));
    }
    github_asset_digest(target, archive).await
}

pub(super) fn checksum_for(checksums: &[u8], archive: &str) -> Option<String> {
    let checksums = std::str::from_utf8(checksums).ok()?;
    checksums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let hash = fields.next()?;
        let name = fields.next()?.trim_start_matches('*');
        (name == archive && valid_checksum(hash)).then(|| hash.to_ascii_lowercase())
    })
}

fn valid_checksum(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

async fn github_asset_digest(target: &Release, archive: &str) -> Result<String, Error> {
    let endpoint = format!("{RELEASE_API}/releases/tags/v{}", target.as_string());
    let response = fetch(&endpoint, "read published daemon checksum").await?;
    let release: serde_json::Value = serde_json::from_slice(&response).map_err(|error| {
        Error::Verification(format!("published release metadata is invalid: {error}"))
    })?;
    let digest = release
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .and_then(|assets| {
            assets.iter().find_map(|asset| {
                (asset.get("name").and_then(serde_json::Value::as_str) == Some(archive))
                    .then(|| asset.get("digest").and_then(serde_json::Value::as_str))
                    .flatten()
            })
        })
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|digest| valid_checksum(digest))
        .ok_or_else(|| {
            Error::Verification(format!(
                "published release metadata has no SHA-256 digest for {archive}"
            ))
        })?;
    Ok(digest.to_ascii_lowercase())
}

fn verify_checksum(path: &Path, expected: &str) -> Result<(), Error> {
    let bytes = fs::read(path).map_err(|source| Error::Io {
        stage: "read staged daemon archive",
        source,
    })?;
    let actual = hex::encode(Sha256::digest(bytes));
    if actual == expected {
        Ok(())
    } else {
        Err(Error::Verification(format!(
            "{} checksum was {actual}, expected {expected}",
            path.file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("daemon archive")
        )))
    }
}

fn extract_archive(archive: &Path, destination: &Path) -> Result<(), Error> {
    let mut command = Command::new("tar");
    command
        .args(["-xzf"])
        .arg(archive)
        .arg("-C")
        .arg(destination);
    run_command("extract staged daemon archive", &mut command)?;
    Ok(())
}

fn verify_executable(path: &Path, target: &Release) -> Result<(), Error> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|source| Error::Io {
        stage: "mark staged daemon executable",
        source,
    })?;
    let mut command = Command::new(path);
    command.arg("version");
    let output = run_command("preflight staged daemon", &mut command)?;
    let observed = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if observed == target.as_string() {
        Ok(())
    } else {
        Err(Error::Verification(format!(
            "staged daemon reported version {observed:?}, expected {}",
            target.as_string()
        )))
    }
}

fn verify_uninstall(path: &Path) -> Result<(), Error> {
    let metadata = path.metadata().map_err(|source| Error::Io {
        stage: "preflight staged uninstall command",
        source,
    })?;
    if metadata.is_file() && metadata.len() != 0 {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|source| Error::Io {
            stage: "mark staged uninstall command executable",
            source,
        })
    } else {
        Err(Error::Verification(
            "daemon archive does not contain a usable ployz-uninstall command".into(),
        ))
    }
}

pub(super) struct Staging {
    pub(super) path: PathBuf,
}

impl Staging {
    pub(super) fn new(parent: &Path) -> Result<Self, Error> {
        fs::create_dir_all(parent).map_err(|source| Error::Io {
            stage: "create daemon installation directory",
            source,
        })?;
        for _ in 0..100 {
            let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(".ployz-stage-{}-{sequence}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(
                        |source| Error::Io {
                            stage: "protect daemon staging directory",
                            source,
                        },
                    )?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(Error::Io {
                        stage: "create daemon staging directory",
                        source,
                    });
                }
            }
        }
        Err(Error::Verification(
            "could not create a private daemon staging directory".into(),
        ))
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(super) fn write_private(path: &Path, bytes: &[u8], stage: &'static str) -> Result<(), Error> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|source| Error::Io { stage, source })?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| Error::Io { stage, source })
}

fn activate(daemon: &Path, uninstall: &Path, paths: &InstallPaths) -> Result<(), Error> {
    let installed = paths.bin_dir.join("ployzd");
    let previous = paths.bin_dir.join("ployzd.previous");
    match fs::remove_file(&previous) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(Error::Io {
                stage: "remove previous daemon release",
                source,
            });
        }
    }
    if installed.exists() {
        fs::hard_link(&installed, &previous).map_err(|source| Error::Io {
            stage: "retain previous daemon release",
            source,
        })?;
    }
    fs::rename(daemon, &installed).map_err(|source| Error::Io {
        stage: "activate verified daemon",
        source,
    })?;
    fs::rename(uninstall, paths.bin_dir.join("ployz-uninstall")).map_err(|source| Error::Io {
        stage: "activate uninstall command",
        source,
    })?;
    File::open(&paths.bin_dir)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| Error::Io {
            stage: "persist daemon activation",
            source,
        })
}
