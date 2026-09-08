//! The pinned BuildKit builder: created for an attempt, removed afterwards,
//! and leaving its dedicated cache volume behind for the next one.

use std::{
    collections::BTreeMap,
    fs,
    io::{Seek as _, SeekFrom, Write as _},
    os::unix::fs::OpenOptionsExt as _,
    path::Path,
};

use serde::Deserialize;

use crate::{BUILDKIT_IMAGE, BuildError, Docker, Streams, builder_name};

/// Exclusive use of the Ployz builder and its retained cache for one attempt.
pub(crate) struct Builder<'a> {
    docker: &'a Docker<'a>,
    name: String,
    lock: Lock,
}

impl<'a> Builder<'a> {
    /// Take the builder, provisioning the pinned BuildKit version if needed.
    ///
    /// # Errors
    /// Fails when Buildx is unavailable or the builder cannot be provisioned.
    pub(crate) fn acquire(docker: &'a Docker<'a>) -> Result<Self, BuildError> {
        Self::acquire_in(docker, &std::env::temp_dir())
    }

    fn acquire_in(docker: &'a Docker<'a>, directory: &Path) -> Result<Self, BuildError> {
        let name = builder_name();
        let lock = Lock::acquire(directory, &name)?;
        // An attempt that never recorded its end may have left work running in
        // the builder. Replace the container instead of assuming it stopped;
        // either way its cache volume survives.
        let unfinished = lock.unfinished();
        let reusable = match existing(docker, &name)? {
            Some(image) if image == BUILDKIT_IMAGE && !unfinished => true,
            Some(_) => {
                remove(&docker.releasing(), &name)?;
                false
            }
            None => false,
        };
        if !reusable {
            create(docker, &name)?;
        }
        lock.begin()?;
        Ok(Self { docker, name, lock })
    }

    /// Run one build, bounded by the attempt's deadline.
    ///
    /// # Errors
    /// Returns BuildKit's own diagnosis, a timeout once the builder is
    /// confirmed stopped, or an uncertain termination when it is not.
    pub(crate) fn run(&self, arguments: &[String]) -> Result<(), BuildError> {
        let borrowed = arguments.iter().map(String::as_str).collect::<Vec<_>>();
        match self.docker.run("the build", &borrowed, Streams::Inherited) {
            Ok(_) => Ok(()),
            Err(BuildError::TimedOut(seconds)) => Err(self.terminate(seconds)),
            Err(error) => Err(error),
        }
    }

    /// The attempt was terminated; report whether its builder stopped too.
    fn terminate(&self, seconds: u64) -> BuildError {
        match remove(&self.docker.releasing(), &self.name) {
            Ok(()) => BuildError::TimedOut(seconds),
            Err(error) => BuildError::UncertainTermination(error.to_string()),
        }
    }
}

impl Drop for Builder<'_> {
    /// Remove the container, keeping the cache volume it was built with, and
    /// record the attempt as finished only once that removal is observed.
    fn drop(&mut self) {
        if remove(&self.docker.releasing(), &self.name).is_ok() {
            self.lock.finish();
        }
    }
}

fn create(docker: &Docker<'_>, name: &str) -> Result<(), BuildError> {
    let image = format!("image={BUILDKIT_IMAGE}");
    // Host networking keeps builds reaching the hosts they reached while
    // Docker built them with its own embedded BuildKit.
    docker
        .run(
            "create the build container",
            &[
                "buildx",
                "create",
                "--name",
                name,
                "--driver",
                "docker-container",
                "--driver-opt",
                &image,
                "--driver-opt",
                "network=host",
            ],
            Streams::Captured,
        )
        .map(|_| ())
}

fn remove(docker: &Docker<'_>, name: &str) -> Result<(), BuildError> {
    match docker.run(
        "remove the build container",
        &["buildx", "rm", "--keep-state", name],
        Streams::Captured,
    ) {
        Ok(_) => Ok(()),
        // Already gone is the state this asks for.
        Err(BuildError::Docker { diagnostic, .. }) if diagnostic.contains("no builder") => Ok(()),
        Err(error) => Err(error),
    }
}

/// The BuildKit image of an existing Ployz builder, when Docker has one.
fn existing(docker: &Docker<'_>, name: &str) -> Result<Option<String>, BuildError> {
    let listed = docker
        .run(
            "list builders",
            &["buildx", "ls", "--format", "{{json .}}"],
            Streams::Captured,
        )
        .map_err(|error| {
            BuildError::Prerequisite(format!(
                "local Builds require Docker with the Buildx plugin: {error}"
            ))
        })?;
    for line in listed.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(instance) = serde_json::from_str::<Instance>(line) else {
            continue;
        };
        if instance.name != name {
            continue;
        }
        return Ok(Some(
            instance
                .nodes
                .unwrap_or_default()
                .into_iter()
                .next()
                .and_then(|node| node.driver_opts.unwrap_or_default().remove("image"))
                .unwrap_or_default(),
        ));
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
    #[serde(rename = "DriverOpts")]
    driver_opts: Option<BTreeMap<String, String>>,
}

/// Serializes local attempts sharing one builder and its retained cache, and
/// records an attempt in flight so one that never recorded its end cannot be
/// mistaken for a finished one. One file, one lock, one state.
struct Lock {
    file: fs::File,
}

impl Lock {
    fn acquire(directory: &Path, name: &str) -> Result<Self, BuildError> {
        use rustix::fs::{FlockOperation, flock};

        let path = directory.join(format!("{name}.lock"));
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|error| {
                BuildError::Prerequisite(format!("open the build lock {}: {error}", path.display()))
            })?;
        if flock(&file, FlockOperation::NonBlockingLockExclusive).is_err() {
            eprintln!("Waiting for another local Ployz build to finish.");
            flock(&file, FlockOperation::LockExclusive).map_err(|error| {
                BuildError::Prerequisite(format!(
                    "wait for another local Ployz build: {}",
                    std::io::Error::from(error)
                ))
            })?;
        }
        Ok(Self { file })
    }

    /// Whether an earlier attempt never recorded its end.
    fn unfinished(&self) -> bool {
        self.file
            .metadata()
            .map_or(true, |metadata| metadata.len() > 0)
    }

    /// Record that an attempt now owns the builder.
    ///
    /// # Errors
    /// Fails when the record cannot be written, which would leave a later
    /// attempt unable to tell an interrupted builder from a finished one.
    fn begin(&self) -> Result<(), BuildError> {
        (&self.file)
            .seek(SeekFrom::Start(0))
            .and_then(|_| (&self.file).write_all(b"running"))
            .map_err(|error| {
                BuildError::Prerequisite(format!("record the build in progress: {error}"))
            })
    }

    fn finish(&self) {
        let _ = self.file.set_len(0);
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    /// A Docker stand-in that records calls and reports one existing builder.
    fn fixture(directory: &Path, listed: &str) -> fs::File {
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}/calls'\ncase \"$1 $2\" in\n  'buildx ls') printf '%s\\n' '{listed}' ;;\nesac\nexit 0\n",
            directory.display()
        );
        let path = directory.join("docker");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        fs::File::open(&path).unwrap()
    }

    fn calls(directory: &Path) -> String {
        fs::read_to_string(directory.join("calls")).unwrap_or_default()
    }

    #[test]
    fn a_builder_left_by_an_unfinished_attempt_is_replaced_before_reuse() {
        let directory = std::env::temp_dir().join(format!("ployz-builder-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let name = builder_name();
        let listed = format!(
            r#"{{"Name":"{name}","Nodes":[{{"DriverOpts":{{"image":"{BUILDKIT_IMAGE}"}}}}]}}"#
        );
        drop(fixture(&directory, &listed));
        let environment = BTreeMap::new();
        let program = directory.join("docker");
        let docker = Docker {
            program: &program,
            environment: &environment,
            working_dir: &directory,
            deadline: crate::Deadline::starting_now(crate::EXECUTION_TIMEOUT),
        };

        // A finished attempt leaves the pinned builder in place for reuse.
        let builder = Builder::acquire_in(&docker, &directory).unwrap();
        assert!(!calls(&directory).contains("buildx create"));
        drop(builder);
        assert!(calls(&directory).contains("buildx rm --keep-state"));

        // An attempt that never recorded its end leaves the builder replaced.
        let lock = Lock::acquire(&directory, &name).unwrap();
        lock.begin().unwrap();
        drop(lock);
        fs::write(directory.join("calls"), "").unwrap();
        let builder = Builder::acquire_in(&docker, &directory).unwrap();
        let recorded = calls(&directory);
        assert!(recorded.contains("buildx rm --keep-state"), "{recorded}");
        assert!(recorded.contains("buildx create --name"), "{recorded}");
        drop(builder);

        fs::remove_dir_all(&directory).unwrap();
    }
}
