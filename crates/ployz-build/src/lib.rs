//! Shared BuildKit execution for one captured Build.
//!
//! A caller captures a Build's inputs, then executes that capture here. Builder
//! setup, subprocess orchestration, image import, output verification, and
//! cleanup stay private so no caller has to restate image-correctness rules.
//! Execution knows source, recipe, and platforms; it knows nothing about
//! Compose selection, Cluster placement, or Deploy.

mod builder;

use std::{
    collections::BTreeMap,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use serde::Deserialize;
use thiserror::Error;

use builder::Builder;

/// Pinned BuildKit release. Every Ployz Build runs this version.
pub const BUILDKIT_IMAGE: &str = "moby/buildkit:v0.26.2";

/// Ployz-owned builder. Its cache volume outlives the container it names.
pub const BUILDER: &str = "ployz";

/// Longest one Build Attempt may run, covering preparation, compilation,
/// and image import.
pub const EXECUTION_TIMEOUT: Duration = Duration::from_secs(30 * 60);

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
    pub name: String,
    /// Requested platforms. Empty selects the execution host's platform.
    pub platforms: Vec<String>,
}

/// What an attempt does with its result. These outcomes are exclusive: an
/// attempt cannot both validate and produce an image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Output {
    /// Load completed images into the execution host's Docker image store.
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

/// A completed image, identified by the content this attempt observed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltImage {
    /// Target name the caller supplied.
    pub target: String,
    /// Digest-pinned reference. Binds later use to this attempt's content, so
    /// a later Build moving a shared tag cannot substitute its image.
    pub reference: String,
    /// Tags this attempt applied, as the execution host recorded them.
    pub tags: Vec<String>,
    /// Platforms actually present, not the ones requested.
    pub platforms: Vec<String>,
    pub location: Location,
}

/// Where a completed image is available.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Location {
    // ponytail: selected-Machine locations arrive with remote execution.
    /// The execution host's local Docker image store.
    Local,
}

impl std::fmt::Display for Location {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local => formatter.write_str("local Docker"),
        }
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BuildError {
    #[error("{0}")]
    Prerequisite(String),
    #[error(
        "'{target}' requests {count} build platforms; a Dockerfile Build produces one platform"
    )]
    MultiplePlatforms { target: String, count: usize },
    #[error("unsupported build platform '{0}'")]
    UnsupportedPlatform(String),
    #[error("the builder cannot build '{platform}'; it supports {available}")]
    UnavailablePlatform { platform: String, available: String },
    #[error("build exited with {0}")]
    Failed(String),
    #[error("build exceeded the {0}s execution timeout")]
    TimedOut(u64),
    #[error(
        "build termination could not be confirmed: {0}. Retained builder state stays unavailable until it is released"
    )]
    UncertainTermination(String),
    #[error("Docker {action}: {diagnostic}")]
    Docker {
        action: &'static str,
        diagnostic: String,
    },
    #[error("{0}")]
    Result(String),
}

/// Execute one captured Build against local Docker.
///
/// Requires Docker with Buildx and the containerd image store. It does not
/// require a Ployz daemon.
///
/// # Errors
/// Returns a refusal for unsupported platform requests, a prerequisite error
/// when Docker or its builder cannot serve the Build, a failure carrying the
/// earliest proven stage, or an uncertain outcome when termination could not
/// be confirmed.
pub fn execute(request: &Request<'_>) -> Result<Outcome, BuildError> {
    if request.targets.is_empty() {
        return Ok(Outcome::Built(Vec::new()));
    }
    // Refuse unsupported platform requests before starting any execution.
    let requested = requested_platforms(request.targets)?;
    let docker = Docker {
        program: request.docker.unwrap_or_else(|| Path::new("docker")),
        environment: request.environment,
        working_dir: request.working_dir,
    };
    let platforms = resolve_platforms(requested, &docker)?;
    let builder = Builder::acquire(&docker)?;
    builder.supports(&platforms)?;
    let metadata = request.working_dir.join("build-metadata.json");
    builder.run(
        &bake_arguments(request, &platforms, &metadata),
        EXECUTION_TIMEOUT,
    )?;
    match request.output {
        Output::Validate => Ok(Outcome::Validated),
        Output::Registry => Ok(Outcome::Published),
        Output::Load => {
            built_images(&docker, &metadata, request.targets, &platforms).map(Outcome::Built)
        }
    }
}

fn bake_arguments(
    request: &Request<'_>,
    platforms: &BTreeMap<String, String>,
    metadata: &Path,
) -> Vec<String> {
    let mut arguments = vec![
        "buildx".to_owned(),
        "bake".to_owned(),
        "--builder".to_owned(),
        BUILDER.to_owned(),
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
    for (target, platform) in platforms {
        arguments.push("--set".to_owned());
        arguments.push(format!("{target}.platform={platform}"));
    }
    for argument in request.build_args {
        arguments.push("--set".to_owned());
        arguments.push(format!("*.args.{argument}"));
    }
    arguments.extend(
        request
            .targets
            .iter()
            .map(|target| bake_target(&target.name)),
    );
    arguments
}

/// Compose service names may contain a dot; Buildx target names may not.
fn bake_target(name: &str) -> String {
    name.replace('.', "_")
}

fn requested_platforms(targets: &[Target]) -> Result<BTreeMap<String, Option<String>>, BuildError> {
    targets
        .iter()
        .map(|target| match target.platforms.as_slice() {
            [] => Ok((bake_target(&target.name), None)),
            [platform] => Ok((
                bake_target(&target.name),
                Some(validated_platform(platform)?),
            )),
            platforms => Err(BuildError::MultiplePlatforms {
                target: target.name.clone(),
                count: platforms.len(),
            }),
        })
        .collect()
}

fn resolve_platforms(
    requested: BTreeMap<String, Option<String>>,
    docker: &Docker<'_>,
) -> Result<BTreeMap<String, String>, BuildError> {
    if requested.values().all(Option::is_some) {
        return Ok(requested
            .into_iter()
            .filter_map(|(target, platform)| Some((target, platform?)))
            .collect());
    }
    let host = host_platform(docker)?;
    Ok(requested
        .into_iter()
        .map(|(target, platform)| (target, platform.unwrap_or_else(|| host.clone())))
        .collect())
}

/// The execution host's own Linux platform, used when none is configured.
fn host_platform(docker: &Docker<'_>) -> Result<String, BuildError> {
    let reported = docker
        .output(
            "read the server platform",
            &["version", "--format", "{{.Server.Os}}/{{.Server.Arch}}"],
        )
        .map_err(|error| {
            BuildError::Prerequisite(format!(
                "local Builds require a reachable Docker daemon: {error}"
            ))
        })?;
    let platform = reported.trim().to_owned();
    if platform.starts_with("linux/") {
        validated_platform(&platform)
    } else {
        Err(BuildError::Prerequisite(format!(
            "Ployz builds Linux images; this Docker host reports '{platform}'"
        )))
    }
}

fn validated_platform(platform: &str) -> Result<String, BuildError> {
    let components = platform.split('/').collect::<Vec<_>>();
    let supported = matches!(components.len(), 2 | 3)
        && components.first() == Some(&"linux")
        && components.iter().all(|component| {
            !component.is_empty()
                && component.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'_' | b'-')
                })
        });
    if supported {
        Ok(platform.to_owned())
    } else {
        Err(BuildError::UnsupportedPlatform(platform.to_owned()))
    }
}

fn built_images(
    docker: &Docker<'_>,
    metadata: &Path,
    targets: &[Target],
    platforms: &BTreeMap<String, String>,
) -> Result<Vec<BuiltImage>, BuildError> {
    let content = std::fs::read(metadata)
        .map_err(|error| BuildError::Result(format!("read the build result metadata: {error}")))?;
    let results: BTreeMap<String, serde_json::Value> = serde_json::from_slice(&content)
        .map_err(|error| BuildError::Result(format!("parse the build result metadata: {error}")))?;
    targets
        .iter()
        .map(|target| {
            let key = bake_target(&target.name);
            let result = results
                .get(&key)
                .ok_or_else(|| {
                    BuildError::Result(format!(
                        "the build reported no result for '{}'",
                        target.name
                    ))
                })
                .and_then(|value| {
                    serde_json::from_value::<TargetMetadata>(value.clone()).map_err(|error| {
                        BuildError::Result(format!(
                            "the build result for '{}' is incomplete: {error}",
                            target.name
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
                    BuildError::Result(format!("the build tagged no image for '{}'", target.name))
                })?
                .to_owned();
            let reference = format!("{repository}@{}", result.digest);
            let requested = platforms.get(&key).map(String::as_str).unwrap_or_default();
            let observed = verify(docker, &reference, &result.digest, requested)?;
            Ok(BuiltImage {
                target: target.name.clone(),
                reference,
                tags,
                platforms: vec![observed],
                location: Location::Local,
            })
        })
        .collect()
}

/// Confirm the execution host holds exactly the content this attempt claims.
fn verify(
    docker: &Docker<'_>,
    reference: &str,
    digest: &str,
    requested: &str,
) -> Result<String, BuildError> {
    let inspected = docker
        .output("inspect the completed image", &[
            "image",
            "inspect",
            reference,
            "--format",
            "{{json .}}",
        ])
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
    // A request without a variant accepts the platform's own variant.
    let matches = observed == requested
        || (requested.split('/').count() == 2
            && observed.starts_with(requested)
            && observed
                .get(requested.len()..)
                .is_some_and(|rest| rest.starts_with('/')));
    if matches {
        Ok(observed)
    } else {
        Err(BuildError::Result(format!(
            "{reference} contains {observed}, not the requested {requested}"
        )))
    }
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

/// The Docker CLI this attempt drives, with the values captured for it.
pub(crate) struct Docker<'a> {
    program: &'a Path,
    environment: &'a BTreeMap<String, String>,
    /// The private capture directory. Running there keeps a stray Compose or
    /// `.env` file in the caller's directory out of the build.
    working_dir: &'a Path,
}

impl Docker<'_> {
    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(self.program);
        command
            .env_clear()
            .envs(self.environment)
            .current_dir(self.working_dir)
            .args(arguments);
        command
    }

    /// Run a command that reports evidence, capturing its output.
    fn output(&self, action: &'static str, arguments: &[&str]) -> Result<String, BuildError> {
        let output = self
            .command(arguments)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| BuildError::Docker {
                action,
                diagnostic: error.to_string(),
            })?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(BuildError::Docker {
                action,
                diagnostic: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }

    /// Run a command whose progress belongs to the operator's terminal.
    fn spawn(&self, arguments: &[String]) -> Result<std::process::Child, BuildError> {
        let borrowed = arguments.iter().map(String::as_str).collect::<Vec<_>>();
        self.command(&borrowed)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|error| BuildError::Docker {
                action: "start the build",
                diagnostic: error.to_string(),
            })
    }

    fn status(&self, action: &'static str, arguments: &[&str]) -> Result<(), BuildError> {
        // Builder lifecycle progress belongs on stderr with the build's own.
        let status = self
            .command(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .map_err(|error| BuildError::Docker {
                action,
                diagnostic: error.to_string(),
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(BuildError::Docker {
                action,
                diagnostic: format!("exited with {status}"),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str, platforms: &[&str]) -> Target {
        Target {
            name: name.to_owned(),
            platforms: platforms
                .iter()
                .map(|platform| (*platform).to_owned())
                .collect(),
        }
    }

    #[test]
    fn one_platform_is_kept_and_several_are_refused_before_execution() {
        let resolved =
            requested_platforms(&[target("api", &["linux/arm64"]), target("web", &[])]).unwrap();
        assert_eq!(resolved.get("api").unwrap().as_deref(), Some("linux/arm64"));
        assert_eq!(resolved.get("web").unwrap().as_deref(), None);
        assert_eq!(
            requested_platforms(&[target("api", &["linux/amd64", "linux/arm64"])]).unwrap_err(),
            BuildError::MultiplePlatforms {
                target: "api".into(),
                count: 2,
            }
        );
        for refused in ["windows/amd64", "linux", "linux//v8", "LINUX/AMD64"] {
            assert_eq!(
                requested_platforms(&[target("api", &[refused])]).unwrap_err(),
                BuildError::UnsupportedPlatform(refused.into()),
            );
        }
        assert_eq!(validated_platform("linux/arm/v7").unwrap(), "linux/arm/v7");
    }

    #[test]
    fn a_dotted_service_keeps_one_buildx_target_name() {
        assert_eq!(bake_target("api.internal"), "api_internal");
        let resolved = requested_platforms(&[target("api.internal", &["linux/amd64"])]).unwrap();
        assert!(resolved.contains_key("api_internal"));
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
        let targets = [target("api", &[])];
        let platforms = BTreeMap::from([("api".to_owned(), "linux/amd64".to_owned())]);
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
        let validate = bake_arguments(&request(Output::Validate), &platforms, metadata);
        assert!(validate.contains(&"--check".to_owned()));
        assert!(!validate.contains(&"--load".to_owned()));
        assert!(!validate.contains(&"--metadata-file".to_owned()));
        let load = bake_arguments(&request(Output::Load), &platforms, metadata);
        assert!(load.contains(&"--load".to_owned()));
        assert!(!load.contains(&"--push".to_owned()));
        assert!(load.contains(&"--no-cache".to_owned()));
        assert!(!load.contains(&"--pull".to_owned()));
        assert!(load.contains(&"api.platform=linux/amd64".to_owned()));
        assert!(load.contains(&"*.args.MODE=release".to_owned()));
        assert_eq!(load.last().map(String::as_str), Some("api"));
        let registry = bake_arguments(&request(Output::Registry), &platforms, metadata);
        assert!(registry.contains(&"--push".to_owned()));
        assert!(!registry.contains(&"--load".to_owned()));
        assert!(!registry.contains(&"--metadata-file".to_owned()));
    }
}
