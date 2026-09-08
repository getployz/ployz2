use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use ployz_build::{BuiltImage, Output};
use ployz_core::MachineTarget;
use serde::Serialize;
use serde_norway::Value;

use super::{BuildSpec, ComposeError, ComposeProject, LoadOptions, build_inputs::BuildInputs};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BuildOptions {
    pub build_args: Vec<String>,
    pub check: bool,
    pub deps: bool,
    pub no_cache: bool,
    pub pull: bool,
    pub push_registry: bool,
    pub services: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BuildService {
    pub name: String,
    pub image: String,
    pub build: Value,
    pub machines: Vec<MachineTarget>,
}

pub fn plan_build(
    project: &ComposeProject,
    options: &BuildOptions,
) -> Result<Vec<BuildService>, ComposeError> {
    if options.services.is_empty() {
        return project
            .builds
            .keys()
            .map(|name| build_service(project, name))
            .collect();
    }

    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    for name in &options.services {
        include_service(
            project,
            name,
            options.deps,
            &mut visiting,
            &mut seen,
            &mut selected,
        )?;
    }
    selected
        .into_iter()
        .filter(|name| project.builds.contains_key(*name))
        .map(|name| build_service(project, name))
        .collect()
}

/// A build whose configuration, options, and local sources have already been captured.
pub struct CapturedBuild {
    plan: Vec<BuildService>,
    targets: Vec<ployz_build::Target>,
    options: BuildOptions,
    environment: BTreeMap<String, String>,
    compose: PathBuf,
    inputs: BuildInputs,
}

/// A Service whose image this command built, bound to the content produced.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltService {
    pub name: String,
    /// Reference the Service requested, used when the image is published.
    pub image: String,
    pub machines: Vec<MachineTarget>,
    pub built: BuiltImage,
}

/// What executing a captured build produced. A validation cannot claim an image.
#[derive(Clone, Debug, PartialEq)]
pub enum BuildOutcome {
    Built(Vec<BuiltService>),
    /// Published to the registry each tag names; no local image is claimed.
    Published,
    /// Validation only; no image was produced.
    Validated,
}

/// Compose build settings this runner refuses rather than silently dropping.
const UNSUPPORTED: [(&str, &str); 6] = [
    ("cache_from", "filesystem build cache import"),
    ("cache_to", "filesystem build cache export"),
    ("network", "host-specific build networking"),
    ("isolation", "host-specific build isolation"),
    ("privileged", "privileged builds"),
    ("x-bake", "custom builder and exporter overrides"),
];

/// Freeze build sources, options, and provider values before invoking Docker.
///
/// # Errors
/// Rejects invalid build inputs, provider failures, and unreadable or unstable source files.
pub fn capture_build(
    plan: &[BuildService],
    options: &BuildOptions,
    project: &mut ComposeProject,
) -> Result<CapturedBuild, ComposeError> {
    let mut inputs = BuildInputs::new()?;
    let mut plan = plan.to_vec();
    let mut secret_names = BTreeSet::new();
    let mut targets = Vec::new();
    for service in &mut plan {
        let image = service.image.clone();
        let name = service.name.clone();
        let build = service
            .build
            .as_mapping_mut()
            .ok_or_else(|| invalid_build("expected a build mapping"))?;
        for (setting, meaning) in UNSUPPORTED {
            if build.contains_key(Value::String(setting.into())) {
                return Err(invalid_build(&format!(
                    "service '{name}' sets build.{setting}; {meaning} is not supported"
                )));
            }
        }
        targets.push(ployz_build::Target {
            name: name.clone(),
            platforms: build
                .get(Value::String("platforms".into()))
                .and_then(Value::as_sequence)
                .map(|platforms| {
                    platforms
                        .iter()
                        .map(|platform| {
                            platform
                                .as_str()
                                .map(ToOwned::to_owned)
                                .ok_or_else(|| invalid_build("invalid build platform"))
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default(),
        });
        // Explicit build tags replace the Service image upstream; keep both.
        if let Some(tags) = build
            .get_mut(Value::String("tags".into()))
            .and_then(Value::as_sequence_mut)
            && !tags.iter().any(|tag| tag.as_str() == Some(image.as_str()))
        {
            tags.push(Value::String(image));
        }
        if let Some(args) = build
            .get_mut(Value::String("args".into()))
            .and_then(Value::as_mapping_mut)
        {
            args.retain(|key, value| {
                if value.is_null() {
                    let Some(captured) = key.as_str().and_then(|key| project.environment.get(key))
                    else {
                        return false;
                    };
                    *value = Value::String(captured.clone());
                }
                true
            });
        }
        if let Some(ssh) = build
            .get_mut(Value::String("ssh".into()))
            .and_then(Value::as_sequence_mut)
        {
            for key in ssh {
                let (id, path) = key
                    .as_str()
                    .and_then(|key| key.split_once(": ").or_else(|| key.split_once('=')))
                    .ok_or_else(|| {
                        invalid_build(
                            "SSH agent sockets cannot be frozen; use an explicit key file",
                        )
                    })?;
                let paths = path
                    .split(',')
                    .map(|path| {
                        inputs
                            .capture(&project.working_dir.join(path))
                            .map(|path| path.to_string_lossy().into_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                *key = Value::String(format!("{id}={}", paths.join(",")));
            }
        }
        let context = build
            .get(Value::String("context".into()))
            .and_then(Value::as_str)
            .unwrap_or(".")
            .to_owned();
        let dockerfile = (!build.contains_key(Value::String("dockerfile_inline".into()))
            && !is_remote_context(&context))
        .then(|| {
            project.working_dir.join(&context).join(
                build
                    .get(Value::String("dockerfile".into()))
                    .and_then(Value::as_str)
                    .unwrap_or("Dockerfile"),
            )
        });
        let captured = capture_context(
            &context,
            &project.working_dir,
            dockerfile.as_deref(),
            &mut inputs,
        )?;
        build.insert(Value::String("context".into()), Value::String(captured));
        if let Some(source) = dockerfile {
            build.insert(
                Value::String("dockerfile".into()),
                Value::String(inputs.dockerfile(&source)?.to_string_lossy().into_owned()),
            );
        }
        if let Some(contexts) = build.get_mut(Value::String("additional_contexts".into())) {
            match contexts {
                Value::Mapping(contexts) => {
                    for context in contexts.values_mut() {
                        let source = context
                            .as_str()
                            .ok_or_else(|| invalid_build("invalid additional context"))?;
                        *context = Value::String(capture_context(
                            source,
                            &project.working_dir,
                            None,
                            &mut inputs,
                        )?);
                    }
                }
                Value::Sequence(contexts) => {
                    for context in contexts {
                        let (name, source) = context
                            .as_str()
                            .and_then(|value| value.split_once('='))
                            .ok_or_else(|| invalid_build("invalid additional context"))?;
                        *context = Value::String(format!(
                            "{name}={}",
                            capture_context(source, &project.working_dir, None, &mut inputs)?
                        ));
                    }
                }
                Value::Null => {}
                Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Tagged(_) => {
                    return Err(invalid_build("invalid additional contexts"));
                }
            }
        }
        if let Some(names) = build
            .get(Value::String("secrets".into()))
            .and_then(Value::as_sequence)
        {
            for secret in names {
                let name = secret
                    .as_str()
                    .or_else(|| {
                        secret
                            .as_mapping()?
                            .get(Value::String("source".into()))?
                            .as_str()
                    })
                    .ok_or_else(|| invalid_build("invalid build secret"))?;
                secret_names.insert(name.to_owned());
            }
        }
    }
    inputs.verify()?;
    let mut secrets = BTreeMap::new();
    for name in secret_names {
        let value = project.resolve_secret(&name)?;
        let file = inputs.secret(secrets.len(), value)?;
        secrets.insert(
            name,
            BuildSecret {
                file: file.to_string_lossy().into_owned(),
            },
        );
    }
    let mut options = options.clone();
    for argument in &mut options.build_args {
        if !argument.contains('=') {
            let value = project.environment.get(argument).ok_or_else(|| {
                invalid_build("a build argument has no captured environment value")
            })?;
            *argument = format!("{argument}={value}");
        }
    }
    let services = plan
        .iter()
        .map(|service| {
            (
                service.name.as_str(),
                BuildServiceOverride {
                    image: &service.image,
                    build: &service.build,
                },
            )
        })
        .collect();
    // Compose interpolates even normalized input. Escape dollars so captured values
    // cannot pick up a later shell/.env value or lose a literal dollar.
    let yaml = serde_norway::to_string(&BuildOverride { services, secrets })
        .map_err(|error| ComposeError::Io(format!("encode captured build: {error}")))?
        .replace('$', "$$");
    let compose = inputs.compose(&yaml)?;
    Ok(CapturedBuild {
        plan,
        targets,
        options,
        environment: project.environment.clone(),
        compose,
        inputs,
    })
}

impl CapturedBuild {
    /// Build this capture through the shared runner, without reading the
    /// original sources again.
    ///
    /// # Errors
    /// Fails if the runner cannot execute the build or the result cannot be
    /// bound to the content it produced.
    pub fn execute(&self, docker: Option<&Path>) -> Result<BuildOutcome, ComposeError> {
        if self.plan.is_empty() {
            return Ok(BuildOutcome::Built(Vec::new()));
        }
        let outcome = ployz_build::execute(&ployz_build::Request {
            compose_file: &self.compose,
            working_dir: self.inputs.root(),
            environment: &self.environment,
            docker,
            targets: &self.targets,
            build_args: &self.options.build_args,
            output: if self.options.check {
                Output::Validate
            } else if self.options.push_registry {
                Output::Registry
            } else {
                Output::Load
            },
            no_cache: self.options.no_cache,
            pull: self.options.pull,
        })?;
        match outcome {
            ployz_build::Outcome::Validated => Ok(BuildOutcome::Validated),
            ployz_build::Outcome::Published => Ok(BuildOutcome::Published),
            ployz_build::Outcome::Built(images) => self.bind(images).map(BuildOutcome::Built),
        }
    }

    /// Bind each planned Service to the image this attempt actually produced.
    fn bind(&self, images: Vec<BuiltImage>) -> Result<Vec<BuiltService>, ComposeError> {
        self.plan
            .iter()
            .map(|service| {
                let built = images
                    .iter()
                    .find(|image| image.target == service.name)
                    .ok_or_else(|| {
                        invalid_build(&format!(
                            "the build reported no image for service '{}'",
                            service.name
                        ))
                    })?;
                Ok(BuiltService {
                    name: service.name.clone(),
                    image: service.image.clone(),
                    machines: service.machines.clone(),
                    built: built.clone(),
                })
            })
            .collect()
    }
}

/// Capture and build one plan in a single step.
///
/// # Errors
/// Propagates capture refusals and build failures.
pub fn execute_build(
    plan: &[BuildService],
    options: &BuildOptions,
    load: &LoadOptions,
    project: &mut ComposeProject,
) -> Result<BuildOutcome, ComposeError> {
    if plan.is_empty() {
        return Ok(BuildOutcome::Built(Vec::new()));
    }
    capture_build(plan, options, project)?.execute(load.docker.as_deref())
}

fn capture_context(
    source: &str,
    directory: &Path,
    dockerfile: Option<&Path>,
    inputs: &mut BuildInputs,
) -> Result<String, ComposeError> {
    if source.starts_with("service:") {
        return Ok(source.into());
    }
    if source.starts_with("docker-image://") && source.contains("@sha256:") {
        return Ok(source.into());
    }
    if is_remote_context(source) {
        // BuildKit fetches commit-pinned Git contexts without consulting a moving ref.
        let revision = source
            .split_once('#')
            .map(|(_, reference)| reference.split(':').next().unwrap_or(""));
        if revision.is_some_and(|revision| {
            revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Ok(source.into());
        }
        return Err(invalid_build(
            "remote build context must use an immutable Git commit or image digest",
        ));
    }
    Ok(inputs
        .context(&directory.join(source), dockerfile)?
        .to_string_lossy()
        .into_owned())
}

fn is_remote_context(source: &str) -> bool {
    source.contains("://") || source.starts_with("git@") || source.starts_with("service:")
}

fn invalid_build(message: &str) -> ComposeError {
    ComposeError::Invalid(message.into())
}

#[derive(Serialize)]
struct BuildServiceOverride<'a> {
    image: &'a str,
    build: &'a Value,
}

#[derive(Serialize)]
struct BuildSecret {
    file: String,
}

#[derive(Serialize)]
struct BuildOverride<'a> {
    services: BTreeMap<&'a str, BuildServiceOverride<'a>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    secrets: BTreeMap<String, BuildSecret>,
}

fn include_service<'a>(
    project: &'a ComposeProject,
    name: &'a str,
    deps: bool,
    visiting: &mut BTreeSet<&'a str>,
    seen: &mut BTreeSet<&'a str>,
    selected: &mut Vec<&'a str>,
) -> Result<(), ComposeError> {
    if !project.services.contains_key(name) {
        return Err(ComposeError::Invalid(format!("undefined service '{name}'")));
    }
    if visiting.contains(name) {
        return Err(ComposeError::Invalid(format!(
            "build dependency cycle at service '{name}'"
        )));
    }
    if seen.contains(name) {
        return Ok(());
    }
    visiting.insert(name);
    selected.push(name);
    if let Some(build) = project.builds.get(name) {
        for dependency in build.additional_services() {
            include_service(project, dependency, deps, visiting, seen, selected)?;
        }
    }
    if deps {
        for dependency in project.dependencies.get(name).into_iter().flatten() {
            include_service(
                project,
                dependency.service.as_str(),
                true,
                visiting,
                seen,
                selected,
            )?;
        }
    }
    visiting.remove(name);
    seen.insert(name);
    Ok(())
}

fn build_service(project: &ComposeProject, name: &str) -> Result<BuildService, ComposeError> {
    let service = project
        .services
        .get(name)
        .ok_or_else(|| ComposeError::Invalid(format!("undefined service '{name}'")))?;
    Ok(BuildService {
        name: name.to_owned(),
        image: service.container.image.clone(),
        build: project
            .builds
            .get(name)
            .expect("build services come from the build map")
            .raw
            .clone(),
        machines: service.placement.machines.clone(),
    })
}

impl BuildSpec {
    /// Service names referenced as `service:` additional build contexts.
    #[must_use]
    pub(crate) fn additional_services(&self) -> Vec<&str> {
        let Value::Mapping(map) = &self.raw else {
            return Vec::new();
        };
        let Some(contexts) = map.get(Value::String("additional_contexts".into())) else {
            return Vec::new();
        };
        match contexts {
            Value::Mapping(map) => map
                .values()
                .filter_map(Value::as_str)
                .filter_map(|context| context.strip_prefix("service:"))
                .collect(),
            Value::Sequence(values) => values
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|value| value.split_once('=').map(|(_, context)| context))
                .filter_map(|context| context.strip_prefix("service:"))
                .collect(),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Tagged(_) => Vec::new(),
        }
    }
}
