//! Optional ZFS preparation for a fresh Machine.

use std::{
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use ployz_core::StorageChoice;

use super::{
    Error, InstallPaths, command_exists, run_apt, run_apt_with_env, run_command, run_host,
};
use super::{
    host::write_file_atomically,
    release::{Staging, write_private},
};

const ZFS_SMOKE_BYTES: u64 = 128 * 1024 * 1024;
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn prepare_storage(storage: StorageChoice, paths: &InstallPaths) -> Result<(), Error> {
    match storage {
        StorageChoice::None => Ok(()),
        StorageChoice::Zfs => prepare_zfs(paths),
    }
}

fn prepare_zfs(paths: &InstallPaths) -> Result<(), Error> {
    let os = operating_system_id()?;
    if os != "ubuntu" {
        return Err(Error::Command {
            stage: "prepare ZFS storage".into(),
            message: format!(
                "ZFS storage preparation is not supported on {os} yet; use a supported Ubuntu release"
            ),
        });
    }
    let container = container_virtualization();
    if container == "openvz" || (Path::new("/proc/vz").is_dir() && !Path::new("/proc/bc").is_dir())
    {
        return Err(Error::Command {
            stage: "prepare ZFS storage".into(),
            message: "OpenVZ does not allow this Machine to load the host ZFS kernel module".into(),
        });
    }
    if container == "lxc" && lxc_is_unprivileged()? {
        return Err(Error::Command {
            stage: "prepare ZFS storage".into(),
            message:
                "Unprivileged LXC does not allow this Machine to load the host ZFS kernel module"
                    .into(),
        });
    }
    if !command_exists("apt-get") {
        return Err(Error::Command {
            stage: "prepare ZFS storage".into(),
            message: "Ubuntu apt-get is required for ZFS storage preparation".into(),
        });
    }
    let kernel = uname("-r", "read running kernel")?;
    require_host_root_reserve(ZFS_SMOKE_BYTES)?;
    let cap = zfs_arc_max()?;
    persist_zfs_arc_max(paths, cap)?;
    install_zfs_packages(&kernel)?;
    run_host("load ZFS kernel module", "modprobe", ["zfs"])?;
    set_and_verify_zfs_arc_max(cap)?;
    validate_zfs()?;
    println!("ZFS storage preparation validated; no Machine Pool was created");
    Ok(())
}

fn operating_system_id() -> Result<String, Error> {
    let value = fs::read_to_string("/etc/os-release").map_err(|source| Error::Io {
        stage: "identify Linux distribution for ZFS storage preparation",
        source,
    })?;
    value
        .lines()
        .find_map(|line| line.strip_prefix("ID="))
        .map(|id| id.trim_matches('"').to_owned())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Error::Command {
            stage: "prepare ZFS storage".into(),
            message: "Could not identify the Linux distribution for ZFS storage preparation".into(),
        })
}

fn container_virtualization() -> String {
    Command::new("systemd-detect-virt")
        .arg("--container")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn lxc_is_unprivileged() -> Result<bool, Error> {
    let map = fs::read_to_string("/proc/self/uid_map").map_err(|source| Error::Io {
        stage: "inspect LXC user namespace",
        source,
    })?;
    let mut fields = map.split_whitespace();
    Ok(matches!(
        (fields.next(), fields.next()),
        (Some("0"), Some(outside)) if outside != "0"
    ))
}

fn uname(arg: &str, stage: &str) -> Result<String, Error> {
    let output = run_host(stage, "uname", [arg])?;
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if value.is_empty() {
        Err(Error::Command {
            stage: stage.into(),
            message: "returned no value".into(),
        })
    } else {
        Ok(value)
    }
}

fn require_host_root_reserve(allocation: u64) -> Result<(), Error> {
    let output = run_host(
        "inspect host-root capacity for ZFS storage preparation",
        "df",
        ["-B1", "--output=size,avail", "/"],
    )?;
    let line = String::from_utf8_lossy(&output.stdout)
        .lines()
        .last()
        .unwrap_or_default()
        .to_owned();
    let mut fields = line.split_whitespace();
    let parse = |value: Option<&str>| value.and_then(|value| value.parse::<u64>().ok());
    let (Some(size), Some(available), None) =
        (parse(fields.next()), parse(fields.next()), fields.next())
    else {
        return Err(Error::Command {
            stage: "prepare ZFS storage".into(),
            message: "Could not read host-root capacity for ZFS storage preparation".into(),
        });
    };
    let reserve = (size / 4).max(10 * 1024 * 1024 * 1024);
    if available < reserve.saturating_add(allocation) {
        return Err(Error::Command {
            stage: "prepare ZFS storage".into(),
            message: format!(
                "Host root has {available} bytes available; ZFS validation needs {allocation} bytes while preserving the {reserve}-byte host-root reserve"
            ),
        });
    }
    Ok(())
}

fn zfs_arc_max() -> Result<u64, Error> {
    let memory = fs::read_to_string("/proc/meminfo").map_err(|source| Error::Io {
        stage: "read total RAM for ZFS ARC limit",
        source,
    })?;
    let kib = memory
        .lines()
        .find_map(|line| {
            line.strip_prefix("MemTotal:")
                .and_then(|value| value.split_whitespace().next())
        })
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| Error::Command {
            stage: "prepare ZFS storage".into(),
            message: "Could not read total RAM for the ZFS ARC limit".into(),
        })?;
    Ok((kib.saturating_mul(1024) / 4).clamp(256 * 1024 * 1024, 1024 * 1024 * 1024))
}

fn persist_zfs_arc_max(paths: &InstallPaths, cap: u64) -> Result<(), Error> {
    fs::create_dir_all(&paths.modprobe_dir).map_err(|source| Error::Io {
        stage: "create ZFS module configuration directory",
        source,
    })?;
    write_file_atomically(
        &paths.modprobe_dir.join("ployz-zfs.conf"),
        &format!("options zfs zfs_arc_max={cap}\n"),
        "persist ZFS ARC limit",
    )
}

fn package_has_zfs_module(files: &str, kernel: &str) -> bool {
    let marker = format!("/lib/modules/{kernel}/");
    files.lines().any(|line| {
        line.contains(&marker) && (line.ends_with("/zfs.ko") || line.contains("/zfs.ko."))
    })
}

fn install_zfs_packages(kernel: &str) -> Result<(), Error> {
    run_apt("refresh Ubuntu packages for ZFS", ["update", "-qq"])?;
    let mut package = None;
    for candidate in [
        format!("linux-main-modules-zfs-{kernel}"),
        format!("linux-modules-zfs-{kernel}"),
        format!("linux-modules-{kernel}"),
        format!("linux-modules-extra-{kernel}"),
    ] {
        let mut show = Command::new("apt-cache");
        show.args(["show", &candidate]);
        if !show
            .status()
            .map_err(|source| Error::Io {
                stage: "inspect Ubuntu ZFS module packages",
                source,
            })?
            .success()
        {
            continue;
        }
        let files = command_stdout(
            "inspect installed Ubuntu ZFS module package",
            "dpkg-query",
            ["-L", &candidate],
        )?;
        if package_has_zfs_module(&files, kernel) {
            package = Some(candidate);
            break;
        }
        let scratch = Staging::new(Path::new("/var/tmp"))?;
        let mut download = Command::new("apt-get");
        download
            .args(["-o", "DPkg::Lock::Timeout=300", "download", &candidate])
            .current_dir(&scratch.path);
        if !run_command("download Ubuntu ZFS module package", &mut download).is_ok() {
            continue;
        }
        let archive = fs::read_dir(&scratch.path)
            .map_err(|source| Error::Io {
                stage: "inspect downloaded Ubuntu ZFS module package",
                source,
            })?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension() == Some(OsStr::new("deb")));
        let Some(archive) = archive else { continue };
        let mut contents = Command::new("dpkg-deb");
        contents.args(["-c"]).arg(&archive);
        let output = run_command(
            "inspect downloaded Ubuntu ZFS module package",
            &mut contents,
        )?;
        if package_has_zfs_module(&String::from_utf8_lossy(&output.stdout), kernel) {
            package = Some(candidate);
            break;
        }
    }
    let package = package.ok_or_else(|| Error::Command {
        stage: "prepare ZFS storage".into(),
        message: format!(
            "Ubuntu has no packaged ZFS module for the running kernel {kernel}; install a supported Ubuntu kernel and retry"
        ),
    })?;
    run_apt_with_env(
        "install ZFS packages",
        [
            "install",
            "-y",
            "-qq",
            "--no-install-recommends",
            "zfsutils-linux",
            &package,
        ],
    )?;
    let files = command_stdout(
        "verify installed Ubuntu ZFS module package",
        "dpkg-query",
        ["-L", &package],
    )?;
    if package_has_zfs_module(&files, kernel) {
        Ok(())
    } else {
        Err(Error::Command {
            stage: "prepare ZFS storage".into(),
            message: format!(
                "Installed package {package} does not supply the ZFS module for running kernel {kernel}"
            ),
        })
    }
}

fn command_stdout<const N: usize>(
    stage: &str,
    program: &str,
    args: [&str; N],
) -> Result<String, Error> {
    let output = run_host(stage, program, args)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn set_and_verify_zfs_arc_max(cap: u64) -> Result<(), Error> {
    let path = Path::new("/sys/module/zfs/parameters/zfs_arc_max");
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|source| Error::Io {
            stage: "apply ZFS ARC limit",
            source,
        })?;
    writeln!(file, "{cap}").map_err(|source| Error::Io {
        stage: "apply ZFS ARC limit",
        source,
    })?;
    let observed = fs::read_to_string(path).map_err(|source| Error::Io {
        stage: "verify ZFS ARC limit",
        source,
    })?;
    if observed.trim() == cap.to_string() {
        Ok(())
    } else {
        Err(Error::Verification(format!(
            "loaded ZFS module reports zfs_arc_max={} instead of the required {cap}",
            observed.trim()
        )))
    }
}

fn validate_zfs() -> Result<(), Error> {
    require_host_root_reserve(ZFS_SMOKE_BYTES)?;
    let stage = Staging::new(Path::new("/var/tmp"))?;
    let backing = stage.path.join("backing");
    write_private(&backing, b"", "create ZFS smoke backing file")?;
    let pool = format!(
        "ployz-smoke-{}-{}",
        std::process::id(),
        STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let mut pool_created = false;
    let result = (|| {
        let mut allocate = Command::new("fallocate");
        allocate
            .args(["-l", &ZFS_SMOKE_BYTES.to_string()])
            .arg(&backing);
        run_command("preallocate ZFS smoke backing file", &mut allocate)?;
        let mut stat = Command::new("stat");
        stat.args(["-c", "%b %B"]).arg(&backing);
        let output = run_command("verify ZFS smoke backing allocation", &mut stat)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut fields = stdout.split_whitespace();
        let blocks = fields.next().and_then(|value| value.parse::<u64>().ok());
        let block_size = fields.next().and_then(|value| value.parse::<u64>().ok());
        if blocks
            .zip(block_size)
            .is_none_or(|(blocks, size)| blocks.saturating_mul(size) < ZFS_SMOKE_BYTES)
        {
            return Err(Error::Verification(format!(
                "ZFS smoke backing file {} is sparse",
                backing.display()
            )));
        }
        let mut mount = Command::new("findmnt");
        mount.args(["-n", "-o", "TARGET", "-T"]).arg(&backing);
        let output = run_command("verify ZFS smoke backing filesystem", &mut mount)?;
        if String::from_utf8_lossy(&output.stdout).trim() != "/" {
            return Err(Error::Verification(format!(
                "ZFS smoke backing file {} is not on the host root filesystem",
                backing.display()
            )));
        }
        let mut create = Command::new("zpool");
        create
            .args(["create", "-f", "-m", "none", "-o", "cachefile=none", &pool])
            .arg(&backing);
        run_command("create temporary ZFS smoke Pool", &mut create)?;
        pool_created = true;
        run_host(
            "query temporary ZFS smoke Pool",
            "zpool",
            ["list", "-Hp", "-o", "name,size,alloc,free", &pool],
        )?;
        run_host(
            "query temporary ZFS smoke dataset",
            "zfs",
            ["list", "-Hp", "-o", "name,mountpoint", &pool],
        )?;
        Ok(())
    })();
    let cleanup = cleanup_zfs_smoke(&pool, &backing, pool_created);
    match (result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => {
            Err(Error::Verification(format!("{cleanup} (after: {error})")))
        }
    }
}

fn cleanup_zfs_smoke(pool: &str, backing: &Path, pool_created: bool) -> Result<(), Error> {
    let existing_pool = if pool_created {
        true
    } else {
        command_stdout(
            "inspect temporary ZFS smoke Pool after failed creation",
            "zpool",
            ["list", "-H", "-o", "name"],
        )?
        .lines()
        .any(|name| name == pool)
    };
    if existing_pool {
        let mut destroy = Command::new("zpool");
        destroy.args(["destroy", "-f", pool]);
        run_command("destroy temporary ZFS smoke Pool", &mut destroy)?;
    }
    let pools = command_stdout(
        "verify temporary ZFS smoke Pool cleanup",
        "zpool",
        ["list", "-H", "-o", "name"],
    )?;
    if pools.lines().any(|name| name == pool) {
        return Err(Error::Verification(format!(
            "Temporary ZFS smoke Pool {pool} remains after destroy"
        )));
    }
    let datasets = command_stdout(
        "verify temporary ZFS smoke dataset cleanup",
        "zfs",
        ["list", "-H", "-o", "name"],
    )?;
    if datasets.lines().any(|name| {
        name == pool
            || name
                .strip_prefix(pool)
                .is_some_and(|suffix| suffix.starts_with('/'))
    }) {
        return Err(Error::Verification(format!(
            "Temporary ZFS smoke dataset {pool} remains after destroy"
        )));
    }
    fs::remove_file(backing).map_err(|source| Error::Io {
        stage: "remove temporary ZFS smoke backing file",
        source,
    })
}
