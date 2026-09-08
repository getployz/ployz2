//! Shared BuildKit execution for one captured Build.
//!
//! A caller captures a Build's inputs, then executes that capture here:
//! execute, verify the image, clean up. Builder lifecycle, subprocess
//! orchestration, image import, and cleanup stay private so no caller has to
//! restate them. Execution knows source, recipe, and platform; it knows
//! nothing about Compose selection, Cluster placement, or Deploy.
//!
//! BuildKit owns build options and their validation. This crate adds only the
//! rules BuildKit cannot enforce: one bounded attempt, and an image bound to
//! the content that attempt produced.

mod builder;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

use serde::Deserialize;
use thiserror::Error;

use builder::Builder;

/// Pinned BuildKit release. Every Ployz Build runs this version.
pub const BUILDKIT_IMAGE: &str = "moby/buildkit:v0.26.2";

/// Longest one Build Attempt may run, covering builder setup, compilation,
/// image import, and verification.
pub const EXECUTION_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Budget for releasing an attempt's resources once it is over.
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(60);

/// The Ployz builder for this user. Its cache volume outlives the container.
///
/// One builder per user keeps two people sharing a Docker daemon from
/// removing each other's build container.
#[must_use]
pub fn builder_name() -> String {
    format!("ployz-{}", rustix::process::getuid().as_raw())
}

/// One Build's captured inputs, ready to execute.
///
/// Paths point into the caller's private capture; nothing here is read from
/// the original sources again.
#[derive(Debug)]
pub struct Request<'a> {
    /// Captured Compose file describing every target of this Build.
    pub compose_file: &'a Path,
    /// Private directory the build runs in, so no stray file can join it.
    pub working_dir: &'a Path,
    /// Values captured for this command. Later shell state is never consulted.
    pub environment: &'a BTreeMap<String, String>,
    /// Docker CLI to execute, defaulting to `docker` on the search path.
    pub docker: Option<&'a Path>,
    pub targets: &'a [Target],
    /// Effective `KEY=VALUE` build-argument overrides for every target.
    pub build_args: &'a [String],
    pub output: Output,
    pub no_cache: bool,
    pub pull: bool,
}

/// One image to produce, named by the caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Target {
    /// Caller's name for this image. Also names it in the captured Compose file.
    pub name: String,
    /// Platform to build. `None` builds the execution host's own platform.
    pub platform: Option<String>,
}

/// What an attempt does with its result. These outcomes are exclusive: an
/// attempt cannot both validate and produce an image.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Output {
    /// Load completed images into the execution host's Docker image store.
    #[default]
    Load,
    /// Publish to the registry each tag names, retaining no local image.
    Registry,
    /// Validate the recipe without producing an image.
    Validate,
}

/// A terminal Build Attempt result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// Images retained on the execution host, bound to observed content.
    Built(Vec<BuiltImage>),
    /// Published to a registry. No local image is claimed.
    Published,
    /// Validation only. No image was produced and none may be claimed.
    Validated,
}

/// A completed image in the execution host's Docker image store, identified by
/// the content this attempt observed there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltImage {
    /// Target name the caller supplied.
    pub target: String,
    /// Digest-pinned reference. Binds later use to this attempt's content, so
    /// a later Build moving a shared tag cannot substitute its image.
    pub reference: String,
    /// Tags this attempt applied, as the execution host recorded them.
    pub tags: Vec<String>,
    /// The platform actually present, not the one requested.
    pub platform: String,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BuildError {
    /// The execution host cannot run Builds at all.
    #[error("{0}")]
    Prerequisite(String),
    /// One Docker command failed. BuildKit's own diagnosis is the diagnostic.
    #[error("{action} failed: {diagnostic}")]
    Docker {
        action: &'static str,
        diagnostic: String,
    },
    /// The attempt exceeded its bounded execution time and was terminated.
    #[error("the build exceeded its {0}s execution timeout and was terminated")]
    TimedOut(u64),
    /// The attempt stopped, but its termination could not be observed.
    #[error("the build was terminated but its builder did not stop: {0}")]
    UncertainTermination(String),
    /// The build finished, but its result is not the image it claims.
    #[error("{0}")]
    Result(String),
}

/// Execute one captured Build against local Docker.
///
/// Requires Docker with Buildx and the containerd image store. It does not
/// require a Ployz daemon. The whole attempt, including builder setup and
/// cleanup, is bounded by [`EXECUTION_TIMEOUT`].
///
/// # Errors
/// Returns a prerequisite error when Docker cannot serve the Build, the
/// failing command's own diagnosis, a timeout, an uncertain outcome when
/// termination could not be observed, or a result error when the completed
/// image is not the content it claims.
pub fn execute(request: &Request<'_>) -> Result<Outcome, BuildError> {
    let docker = Docker {
        program: request.docker.unwrap_or_else(|| Path::new("docker")),
        environment: request.environment,
        working_dir: request.working_dir,
        deadline: Deadline::starting_now(EXECUTION_TIMEOUT),
    };
    let planned = plan(request.targets)?;
    if planned.is_empty() {
        return Ok(empty(request.output));
    }
    let metadata = request.working_dir.join("build-metadata.json");
    let builder = Builder::acquire(&docker)?;
    builder.run(&bake_arguments(request, &planned, &metadata))?;
    match request.output {
        Output::Validate => Ok(Outcome::Validated),
        Output::Registry => Ok(Outcome::Published),
        Output::Load => built_images(&docker, &metadata, &planned).map(Outcome::Built),
    }
}

/// One target with the Buildx name it will carry, derived once.
#[derive(Debug)]
struct Planned<'a> {
    target: &'a Target,
    bake: String,
}

/// Name each target for Buildx, refusing names that would share one result.
fn plan(targets: &[Target]) -> Result<Vec<Planned<'_>>, BuildError> {
    let mut names = BTreeSet::new();
    targets
        .iter()
        .map(|target| {
            // Compose names may contain a dot; Buildx target names may not.
            let bake = target.name.replace('.', "_");
            if names.insert(bake.clone()) {
                Ok(Planned { target, bake })
            } else {
                Err(BuildError::Result(format!(
                    "'{}' and an earlier target share the build name '{bake}', so their results cannot be told apart",
                    target.name
                )))
            }
        })
        .collect()
}

/// A Build with no target still reports what it did, not an image it lacks.
fn empty(output: Output) -> Outcome {
    match output {
        Output::Load => Outcome::Built(Vec::new()),
        Output::Registry => Outcome::Published,
        Output::Validate => Outcome::Validated,
    }
}

fn bake_arguments(request: &Request<'_>, planned: &[Planned<'_>], metadata: &Path) -> Vec<String> {
    let mut arguments = vec![
        "buildx".to_owned(),
        "bake".to_owned(),
        "--builder".to_owned(),
        builder_name(),
        "--file".to_owned(),
        request.compose_file.to_string_lossy().into_owned(),
    ];
    match request.output {
        // Check runs the frontend only; an image would contradict the result.
        Output::Validate => arguments.push("--check".to_owned()),
        // Publication leaves the image with its registry, so there is no local
        // result to bind and no metadata to read.
        Output::Registry => arguments.push("--push".to_owned()),
        Output::Load => {
            arguments.push("--load".to_owned());
            arguments.push("--metadata-file".to_owned());
            arguments.push(metadata.to_string_lossy().into_owned());
        }
    }
    if request.no_cache {
        arguments.push("--no-cache".to_owned());
    }
    if request.pull {
        arguments.push("--pull".to_owned());
    }
    for planned in planned {
        // Without a requested platform the builder uses its own, as Docker did.
        if let Some(platform) = &planned.target.platform {
            arguments.push("--set".to_owned());
            arguments.push(format!("{}.platform={platform}", planned.bake));
        }
    }
    for argument in request.build_args {
        arguments.push("--set".to_owned());
        arguments.push(format!("*.args.{argument}"));
    }
    arguments.extend(planned.iter().map(|planned| planned.bake.clone()));
    arguments
}

fn built_images(
    docker: &Docker<'_>,
    metadata: &Path,
    planned: &[Planned<'_>],
) -> Result<Vec<BuiltImage>, BuildError> {
    let content = std::fs::read(metadata)
        .map_err(|error| BuildError::Result(format!("read the build result: {error}")))?;
    let results: BTreeMap<String, serde_json::Value> = serde_json::from_slice(&content)
        .map_err(|error| BuildError::Result(format!("parse the build result: {error}")))?;
    planned
        .iter()
        .map(|planned| {
            let name = &planned.target.name;
            let result = results
                .get(&planned.bake)
                .ok_or_else(|| {
                    BuildError::Result(format!("the build reported no result for '{name}'"))
                })
                .and_then(|value| {
                    TargetMetadata::deserialize(value).map_err(|error| {
                        BuildError::Result(format!(
                            "the build result for '{name}' is incomplete: {error}"
                        ))
                    })
                })?;
            let tags = result
                .name
                .split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            let repository = tags
                .first()
                .map(|tag| repository(tag))
                .ok_or_else(|| {
                    BuildError::Result(format!("the build tagged no image for '{name}'"))
                })?
                .to_owned();
            let reference = format!("{repository}@{}", result.digest);
            let platform = verify(
                docker,
                &reference,
                &result.digest,
                planned.target.platform.as_deref(),
            )?;
            Ok(BuiltImage {
                target: name.clone(),
                reference,
                tags,
                platform,
            })
        })
        .collect()
}

/// Confirm the execution host holds exactly the content this attempt claims,
/// and report the platform it actually holds.
fn verify(
    docker: &Docker<'_>,
    reference: &str,
    digest: &str,
    requested: Option<&str>,
) -> Result<String, BuildError> {
    let inspected = docker
        .run(
            "inspect the completed image",
            &["image", "inspect", reference, "--format", "{{json .}}"],
            Streams::Captured,
        )
        .map_err(|error| {
            BuildError::Result(format!(
                "the completed image {reference} is not in the local image store, which Ployz Builds require Docker's containerd image store to provide: {error}"
            ))
        })?;
    let image: ImageInspection = serde_json::from_str(&inspected)
        .map_err(|error| BuildError::Result(format!("read the completed image: {error}")))?;
    // The containerd image store identifies an image by the manifest just
    // built, so a different identity means different content was retained.
    if image.id != digest {
        return Err(BuildError::Result(format!(
            "the local image store holds {} for {reference} rather than the completed content, which Ployz Builds require Docker's containerd image store to retain",
            image.id
        )));
    }
    let variant = image.variant.unwrap_or_default();
    let observed = if variant.is_empty() {
        format!("{}/{}", image.os, image.architecture)
    } else {
        format!("{}/{}/{variant}", image.os, image.architecture)
    };
    match requested {
        Some(requested) if !covers(&observed, requested) => Err(BuildError::Result(format!(
            "{reference} contains {observed}, not the requested {requested}"
        ))),
        Some(_) | None => Ok(observed),
    }
}

/// Whether an observed platform satisfies a request, ignoring an unstated
/// variant: `linux/arm64` asks for whichever variant the host builds.
fn covers(observed: &str, requested: &str) -> bool {
    observed == requested
        || observed
            .strip_prefix(requested)
            .or_else(|| requested.strip_prefix(observed))
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The repository part of a tag, keeping a registry port intact.
fn repository(tag: &str) -> &str {
    let start = tag.rfind('/').map_or(0, |slash| slash + 1);
    match tag.get(start..).and_then(|name| name.find([':', '@'])) {
        Some(offset) => tag.get(..start + offset).unwrap_or(tag),
        None => tag,
    }
}

#[derive(Deserialize)]
struct TargetMetadata {
    #[serde(rename = "containerimage.digest")]
    digest: String,
    #[serde(rename = "image.name")]
    name: String,
}

#[derive(Deserialize)]
struct ImageInspection {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Os")]
    os: String,
    #[serde(rename = "Architecture")]
    architecture: String,
    #[serde(rename = "Variant")]
    variant: Option<String>,
}

/// When this attempt must be over, so no Docker command can outlast it.
#[derive(Clone, Copy)]
pub(crate) struct Deadline {
    expires: Instant,
    budget: Duration,
}

impl Deadline {
    fn starting_now(budget: Duration) -> Self {
        Self {
            expires: Instant::now() + budget,
            budget,
        }
    }

    fn remaining(self) -> Duration {
        self.expires.saturating_duration_since(Instant::now())
    }
}

/// Whether a command's output belongs to the operator or to this crate.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Streams {
    /// Build progress the operator watches.
    Inherited,
    /// Evidence this crate reads.
    Captured,
}

/// The Docker CLI this attempt drives, with the values captured for it.
///
/// Every command runs inside the attempt's deadline, so no phase of a bounded
/// Build can wait forever on Docker.
pub(crate) struct Docker<'a> {
    program: &'a Path,
    environment: &'a BTreeMap<String, String>,
    working_dir: &'a Path,
    deadline: Deadline,
}

impl<'a> Docker<'a> {
    /// The same Docker with a fresh budget for releasing resources, so
    /// cleanup still runs, bounded, after the attempt's deadline passes.
    pub(crate) fn releasing(&self) -> Docker<'a> {
        Docker {
            program: self.program,
            environment: self.environment,
            working_dir: self.working_dir,
            deadline: Deadline::starting_now(CLEANUP_TIMEOUT),
        }
    }

    /// Run one Docker command to completion within the deadline.
    ///
    /// # Errors
    /// Returns the command's own diagnosis, or a timeout after terminating it.
    pub(crate) fn run(
        &self,
        action: &'static str,
        arguments: &[&str],
        streams: Streams,
    ) -> Result<String, BuildError> {
        // Captured output goes to a file, so a large result cannot fill a pipe
        // and deadlock a child this side is no longer reading.
        let captured = self.working_dir.join("docker-output");
        let mut command = Command::new(self.program);
        command
            .env_clear()
            .envs(self.environment)
            .current_dir(self.working_dir)
            .args(arguments)
            .stdin(Stdio::null());
        if streams == Streams::Captured {
            let file = std::fs::File::create(&captured).map_err(|error| BuildError::Docker {
                action,
                diagnostic: error.to_string(),
            })?;
            command.stdout(file).stderr(Stdio::piped());
        }
        let mut child = spawn_retrying_busy(&mut command).map_err(|error| BuildError::Docker {
            action,
            diagnostic: error.to_string(),
        })?;
        let Some(status) =
            wait_bounded(&mut child, self.deadline.remaining()).map_err(|error| {
                BuildError::Docker {
                    action,
                    diagnostic: error.to_string(),
                }
            })?
        else {
            return Err(BuildError::TimedOut(self.deadline.budget.as_secs()));
        };
        if !status.success() {
            let mut diagnostic = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                use std::io::Read as _;
                let _ = stderr.read_to_string(&mut diagnostic);
            }
            let diagnostic = diagnostic.trim();
            return Err(BuildError::Docker {
                action,
                diagnostic: if diagnostic.is_empty() {
                    format!("exited with {status}")
                } else {
                    diagnostic.to_owned()
                },
            });
        }
        match streams {
            Streams::Captured => {
                std::fs::read_to_string(&captured).map_err(|error| BuildError::Docker {
                    action,
                    diagnostic: error.to_string(),
                })
            }
            Streams::Inherited => Ok(String::new()),
        }
    }
}

/// A program written moments ago can still be held open by a concurrent fork.
/// Retry briefly rather than fail the Build on that race.
fn spawn_retrying_busy(command: &mut Command) -> std::io::Result<Child> {
    for attempt in 0..4 {
        match command.spawn() {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(Duration::from_millis(10 << attempt));
            }
            other => return other,
        }
    }
    command.spawn()
}

/// Wait for a child, terminating it when the budget runs out.
fn wait_bounded(child: &mut Child, budget: Duration) -> std::io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + budget;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str, platform: Option<&str>) -> Target {
        Target {
            name: name.to_owned(),
            platform: platform.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn targets_that_would_share_one_build_name_are_refused() {
        let distinct = [target("api.internal", None), target("web", None)];
        let planned = plan(&distinct).unwrap();
        assert_eq!(
            planned
                .iter()
                .map(|planned| planned.bake.as_str())
                .collect::<Vec<_>>(),
            ["api_internal", "web"]
        );
        let colliding = [target("api.internal", None), target("api_internal", None)];
        let collision = match plan(&colliding) {
            Ok(_) => panic!("two targets shared one build name"),
            Err(error) => error.to_string(),
        };
        assert!(collision.contains("api_internal"), "{collision}");
    }

    #[test]
    fn an_observed_platform_covers_a_request_without_its_variant() {
        assert!(covers("linux/arm64", "linux/arm64"));
        assert!(covers("linux/arm64/v8", "linux/arm64"));
        assert!(covers("linux/arm64", "linux/arm64/v8"));
        assert!(!covers("linux/amd64", "linux/arm64"));
        assert!(!covers("linux/arm", "linux/arm64"));
    }

    #[test]
    fn a_repository_survives_tags_digests_and_registry_ports() {
        assert_eq!(
            repository("docker.io/library/api:v1"),
            "docker.io/library/api"
        );
        assert_eq!(repository("127.0.0.1:5000/api:v1"), "127.0.0.1:5000/api");
        assert_eq!(
            repository("registry.test:5000/team/api"),
            "registry.test:5000/team/api"
        );
        assert_eq!(
            repository(
                "api@sha256:0000000000000000000000000000000000000000000000000000000000000000"
            ),
            "api"
        );
    }

    #[test]
    fn requested_output_selects_exclusive_bake_behavior() {
        let environment = BTreeMap::new();
        let targets = [target("api", Some("linux/arm64")), target("web", None)];
        let planned = plan(&targets).unwrap();
        let metadata = Path::new("/private/build-metadata.json");
        let build_args = ["MODE=release".to_owned()];
        let request = |output| Request {
            compose_file: Path::new("/private/compose.yaml"),
            working_dir: Path::new("/private"),
            environment: &environment,
            docker: None,
            targets: &targets,
            build_args: &build_args,
            output,
            no_cache: true,
            pull: false,
        };

        let validate = bake_arguments(&request(Output::Validate), &planned, metadata);
        assert!(validate.contains(&"--check".to_owned()));
        assert!(!validate.contains(&"--load".to_owned()));
        assert!(!validate.contains(&"--metadata-file".to_owned()));

        let load = bake_arguments(&request(Output::Load), &planned, metadata);
        assert!(load.contains(&"--load".to_owned()));
        assert!(!load.contains(&"--push".to_owned()));
        assert!(load.contains(&"--no-cache".to_owned()));
        assert!(!load.contains(&"--pull".to_owned()));
        assert!(load.contains(&"*.args.MODE=release".to_owned()));
        // Only a requested platform is set; the rest build the host's own.
        assert!(load.contains(&"api.platform=linux/arm64".to_owned()));
        assert!(
            !load
                .iter()
                .any(|argument| argument.starts_with("web.platform"))
        );
        assert_eq!(load.last().map(String::as_str), Some("web"));

        let registry = bake_arguments(&request(Output::Registry), &planned, metadata);
        assert!(registry.contains(&"--push".to_owned()));
        assert!(!registry.contains(&"--load".to_owned()));
        assert!(!registry.contains(&"--metadata-file".to_owned()));
    }

    #[test]
    fn a_build_with_no_target_claims_only_what_it_did() {
        assert_eq!(empty(Output::Load), Outcome::Built(Vec::new()));
        assert_eq!(empty(Output::Registry), Outcome::Published);
        assert_eq!(empty(Output::Validate), Outcome::Validated);
    }

    #[test]
    fn a_command_that_outlasts_its_budget_is_terminated() {
        let mut child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let waited = Instant::now();
        assert!(
            wait_bounded(&mut child, Duration::from_millis(200))
                .unwrap()
                .is_none()
        );
        assert!(waited.elapsed() < Duration::from_secs(5));
    }
}
