//! The pinned BuildKit builder: created for an attempt, removed afterwards,
//! and leaving its dedicated cache volume behind for the next one.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, ErrorKind},
    os::unix::fs::OpenOptionsExt as _,
    path::PathBuf,
    process::{Child, ExitStatus},
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::{BUILDER, BUILDKIT_IMAGE, BuildError, Docker};

/// Exclusive use of the Ployz builder and its retained cache for one attempt.
pub(crate) struct Builder<'a> {
    docker: &'a Docker<'a>,
    platforms: Vec<String>,
    lock: Lock,
}

impl<'a> Builder<'a> {
    /// Take the builder, provisioning the pinned BuildKit version if needed.
    ///
    /// # Errors
    /// Fails when Buildx is unavailable, when the builder cannot be created,
    /// or when an earlier attempt left its state unconfirmed.
    pub(crate) fn acquire(docker: &'a Docker<'a>) -> Result<Self, BuildError> {
        let lock = Lock::acquire()?;
        let mut existing = node(docker)?;
        if lock.uncertain() {
            if existing.is_some() {
                return Err(BuildError::UncertainTermination(format!(
                    "an earlier Build left builder '{BUILDER}' unconfirmed; release it with `docker buildx rm --keep-state {BUILDER}`"
                )));
            }
            // The builder is gone, so the earlier attempt is over after all.
            lock.clear();
        }
        // A builder surviving an interrupted run is idle: this lock proves no
        // other local attempt owns it, so it is reused rather than rebuilt.
        if existing
            .as_ref()
            .is_some_and(|node| node.image.as_deref() != Some(BUILDKIT_IMAGE))
        {
            remove(docker)?;
            existing = None;
        }
        if existing.is_none() {
            let image = format!("image={BUILDKIT_IMAGE}");
            // Host networking keeps builds reaching the same hosts they reached
            // when Docker built them with its own embedded BuildKit.
            docker.status(
                "create the build container",
                &[
                    "buildx",
                    "create",
                    "--name",
                    BUILDER,
                    "--driver",
                    "docker-container",
                    "--driver-opt",
                    &image,
                    "--driver-opt",
                    "network=host",
                ],
            )?;
        }
        // Boot the worker so its platforms are observed rather than assumed.
        docker.status(
            "start the build container",
            &["buildx", "inspect", "--bootstrap", "--builder", BUILDER],
        )?;
        let platforms = node(docker)?.map(|node| node.platforms).unwrap_or_default();
        Ok(Self {
            docker,
            platforms,
            lock,
        })
    }

    /// Check requested platforms against the worker that will build them.
    ///
    /// # Errors
    /// Returns the platforms the booted worker offers when one is missing.
    pub(crate) fn supports(&self, platforms: &BTreeMap<String, String>) -> Result<(), BuildError> {
        if self.platforms.is_empty() {
            // No observation, so no refusal: let the build report its own.
            return Ok(());
        }
        for platform in platforms.values() {
            if !self
                .platforms
                .iter()
                .any(|supported| covers(supported, platform))
            {
                return Err(BuildError::UnavailablePlatform {
                    platform: platform.clone(),
                    available: self.platforms.join(", "),
                });
            }
        }
        Ok(())
    }

    /// Run one bounded build, terminating it when the timeout expires.
    ///
    /// # Errors
    /// Returns the build's own failure, the timeout, or an unconfirmed
    /// termination when the builder could not be stopped afterwards.
    pub(crate) fn run(&self, arguments: &[String], timeout: Duration) -> Result<(), BuildError> {
        let mut child = self.docker.spawn(arguments)?;
        match wait_bounded(&mut child, timeout) {
            Ok(Some(status)) if status.success() => Ok(()),
            Ok(Some(status)) => Err(BuildError::Failed(status.to_string())),
            Ok(None) => Err(self.terminate(timeout)),
            Err(error) => Err(BuildError::Docker {
                action: "wait for the build",
                diagnostic: error.to_string(),
            }),
        }
    }

    /// The client stopped waiting; report whether the build itself stopped.
    fn terminate(&self, timeout: Duration) -> BuildError {
        match remove(self.docker) {
            Ok(()) => BuildError::TimedOut(timeout.as_secs()),
            Err(error) => {
                self.lock.mark_uncertain();
                BuildError::UncertainTermination(error.to_string())
            }
        }
    }
}

impl Drop for Builder<'_> {
    /// Remove the container, keeping the cache volume it was built with.
    fn drop(&mut self) {
        let _ = remove(self.docker);
    }
}

/// Whether a worker platform covers a request, ignoring an unstated variant.
fn covers(supported: &str, requested: &str) -> bool {
    supported == requested
        || supported
            .strip_prefix(requested)
            .or_else(|| requested.strip_prefix(supported))
            .is_some_and(|rest| rest.starts_with('/'))
}

fn remove(docker: &Docker<'_>) -> Result<(), BuildError> {
    docker.status(
        "remove the build container",
        &["buildx", "rm", "--keep-state", BUILDER],
    )
}

struct Node {
    platforms: Vec<String>,
    image: Option<String>,
}

/// The Ployz builder's first node, when Docker already has one.
fn node(docker: &Docker<'_>) -> Result<Option<Node>, BuildError> {
    let listed = docker
        .output("list builders", &["buildx", "ls", "--format", "{{json .}}"])
        .map_err(|error| {
            BuildError::Prerequisite(format!(
                "local Builds require Docker with the Buildx plugin: {error}"
            ))
        })?;
    for line in listed.lines().filter(|line| !line.trim().is_empty()) {
        let instance: Instance = match serde_json::from_str(line) {
            Ok(instance) => instance,
            Err(_) => continue,
        };
        if instance.name != BUILDER {
            continue;
        }
        let Some(node) = instance.nodes.unwrap_or_default().into_iter().next() else {
            return Ok(Some(Node {
                platforms: Vec::new(),
                image: None,
            }));
        };
        return Ok(Some(Node {
            platforms: node.platforms.unwrap_or_default(),
            image: node
                .driver_opts
                .unwrap_or_default()
                .get("image")
                .map(ToOwned::to_owned),
        }));
    }
    Ok(None)
}

#[derive(Deserialize)]
struct Instance {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Nodes")]
    nodes: Option<Vec<NodeEntry>>,
}

#[derive(Deserialize)]
struct NodeEntry {
    #[serde(rename = "Platforms")]
    platforms: Option<Vec<String>>,
    #[serde(rename = "DriverOpts")]
    driver_opts: Option<BTreeMap<String, String>>,
}

fn wait_bounded(child: &mut Child, timeout: Duration) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Serializes local attempts sharing one builder and its retained cache, and
/// records an attempt whose termination could not be confirmed.
struct Lock {
    _file: fs::File,
    sentinel: PathBuf,
}

impl Lock {
    fn acquire() -> Result<Self, BuildError> {
        // ponytail: one lock per user; Machine-local build admission is remote work.
        let user = rustix::process::getuid().as_raw();
        let directory = std::env::temp_dir();
        let path = directory.join(format!("ployz-build-{user}.lock"));
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|error| lock_error(&error))?;
        rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
            .map_err(|error| lock_error(&io::Error::from(error)))?;
        Ok(Self {
            _file: file,
            sentinel: directory.join(format!("ployz-build-{user}.unconfirmed")),
        })
    }

    fn uncertain(&self) -> bool {
        match fs::metadata(&self.sentinel) {
            Ok(_) => true,
            Err(error) => error.kind() != ErrorKind::NotFound,
        }
    }

    fn mark_uncertain(&self) {
        let _ = fs::write(&self.sentinel, BUILDER);
    }

    fn clear(&self) {
        let _ = fs::remove_file(&self.sentinel);
    }
}

fn lock_error(error: &io::Error) -> BuildError {
    BuildError::Prerequisite(format!("wait for another local Ployz build: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worker_platform_covers_a_request_without_its_variant() {
        assert!(covers("linux/arm64", "linux/arm64"));
        assert!(covers("linux/arm64", "linux/arm64/v8"));
        assert!(covers("linux/arm64/v8", "linux/arm64"));
        assert!(!covers("linux/amd64", "linux/arm64"));
        assert!(!covers("linux/arm", "linux/arm64"));
    }

    #[test]
    fn an_unconfirmed_attempt_is_recorded_until_it_is_cleared() {
        let lock = Lock::acquire().unwrap();
        lock.clear();
        assert!(!lock.uncertain());
        lock.mark_uncertain();
        assert!(lock.uncertain());
        lock.clear();
        assert!(!lock.uncertain());
    }
}
