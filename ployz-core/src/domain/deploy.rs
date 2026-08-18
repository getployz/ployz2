//! Deploy Intent, Plan Options, Preview, and Outcome shared by the CLI planner and `@ployz/sdk`.

use std::{
    collections::BTreeMap,
    fmt::{self, Display, Formatter},
};

use serde::{Deserialize, Serialize};

use super::{
    ContainerRuntimeObservation, RequestedServiceSpec, ResolvedServiceSpec, ServiceVolume,
};
use crate::{ContainerId, MachineId, ProjectName, QualifiedService, RpcError, ServiceName};
use thiserror::Error;

/// Planner knobs for one Deploy.
///
/// `selected` is the Service list this command applies. Empty means full
/// reconciliation of `DeployIntent.target`. Non-empty means partial. There is
/// no independent prune flag.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlanOptions {
    /// Recreate containers even when the resolved spec matches.
    pub force_recreate: bool,
    /// Skip waiting on container health after start or replace.
    pub skip_health_monitor: bool,
    /// Caller-supplied entropy keeps the planner pure while varying equal-priority placement.
    pub placement_seed: u64,
    /// Service Names this command applies. Empty means full reconciliation.
    #[serde(default)]
    pub selected: Vec<ServiceAttempt>,
}

/// One Service Name this Deploy will apply from the target.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServiceAttempt {
    /// Service Name to apply from `DeployIntent.target`.
    pub name: ServiceName,
}

/// Complete desired Services plus which of those Services this command applies.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeployIntent {
    /// Project that will own Containers this Deploy creates.
    pub project_name: ProjectName,
    /// Complete desired Services for this Cluster.
    pub target: Vec<RequestedServiceSpec>,
    /// Planner knobs for this Deploy, including the selected Service list.
    pub options: PlanOptions,
    // ponytail: planner graph, not wire. Split if Cloud ever sends depends_on.
    #[serde(default, skip)]
    dependencies: BTreeMap<ServiceName, Vec<ServiceName>>,
    #[serde(default, skip)]
    service_profiles: BTreeMap<ServiceName, Vec<String>>,
    #[serde(default, skip)]
    requested_profiles: Vec<String>,
    #[serde(default, skip)]
    compose_refusal: Option<ComposePruneRefusal>,
}

impl DeployIntent {
    /// Complete desired Services and planning options.
    ///
    /// Empty `options.selected` means full reconciliation of `target`. Names in
    /// `selected` that are absent from `target` are not planned; they are not a
    /// prune.
    #[must_use]
    pub fn new(
        project_name: ProjectName,
        target: Vec<RequestedServiceSpec>,
        options: PlanOptions,
    ) -> Self {
        Self {
            project_name,
            target,
            options,
            dependencies: BTreeMap::new(),
            service_profiles: BTreeMap::new(),
            requested_profiles: Vec::new(),
            compose_refusal: None,
        }
    }

    /// Full reconciliation of every spec: `target` is those specs and `selected` stays empty.
    #[must_use]
    pub fn apply_all<'a>(
        project_name: ProjectName,
        specs: impl IntoIterator<Item = &'a RequestedServiceSpec>,
        options: PlanOptions,
    ) -> Self {
        Self::new(project_name, specs.into_iter().cloned().collect(), options)
    }

    /// One-spec target; `selected` is that name, so the Deploy is partial.
    #[must_use]
    pub fn apply_one(
        project_name: ProjectName,
        spec: RequestedServiceSpec,
        mut options: PlanOptions,
    ) -> Self {
        options.selected = vec![ServiceAttempt {
            name: spec.name.clone(),
        }];
        Self::new(project_name, vec![spec], options)
    }

    /// Target from every loaded spec; `options.selected` is this command's Service list.
    #[must_use]
    pub fn from_named_specs(
        project_name: ProjectName,
        services: &BTreeMap<String, RequestedServiceSpec>,
        dependencies: &BTreeMap<String, Vec<String>>,
        options: PlanOptions,
    ) -> Self {
        let dependencies = dependencies
            .iter()
            .filter_map(|(name, deps)| {
                Some((
                    services.get(name)?.name.clone(),
                    deps.iter()
                        .filter_map(|dep| services.get(dep).map(|spec| spec.name.clone()))
                        .collect(),
                ))
            })
            .collect();
        Self::new(project_name, services.values().cloned().collect(), options)
            .with_dependencies(dependencies)
    }

    /// `depends_on` edges used to expand and order `selected` inside the planner.
    #[must_use]
    pub fn with_dependencies(
        mut self,
        dependencies: BTreeMap<ServiceName, Vec<ServiceName>>,
    ) -> Self {
        self.dependencies = dependencies;
        self
    }

    /// Compose `profiles:` per Service Name in `target`.
    #[must_use]
    pub fn with_service_profiles(
        mut self,
        service_profiles: BTreeMap<ServiceName, Vec<String>>,
    ) -> Self {
        self.service_profiles = service_profiles;
        self
    }

    /// Profiles requested for this command. They decide what starts, not what exists.
    #[must_use]
    pub fn with_requested_profiles(mut self, requested_profiles: Vec<String>) -> Self {
        self.requested_profiles = requested_profiles;
        self
    }

    /// Compose-side reason pruning is refused, if any.
    #[must_use]
    pub fn with_compose_refusal(mut self, compose_refusal: Option<ComposePruneRefusal>) -> Self {
        self.compose_refusal = compose_refusal;
        self
    }

    /// Planner `depends_on` edges used to expand and order `selected`.
    #[must_use]
    pub fn dependencies(&self) -> &BTreeMap<ServiceName, Vec<ServiceName>> {
        &self.dependencies
    }

    /// Whether `name` starts given the requested profile list.
    #[must_use]
    pub fn service_starts(&self, name: &ServiceName) -> bool {
        profiles_enable_start(
            self.service_profiles
                .get(name)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            &self.requested_profiles,
        )
    }

    /// Why pruning is refused for this Intent and Snapshot completeness.
    #[must_use]
    pub fn prune_refusal(&self, snapshot_complete: bool) -> Option<PruneRefusal> {
        if !snapshot_complete {
            Some(PruneRefusal::IncompleteSnapshot)
        } else if !self.options.selected.is_empty() {
            Some(PruneRefusal::SelectedServices)
        } else {
            self.compose_refusal.map(PruneRefusal::from)
        }
    }
}

/// Unprofiled Services always start; otherwise any requested profile match starts them.
#[must_use]
pub fn profiles_enable_start(service_profiles: &[String], requested_profiles: &[String]) -> bool {
    service_profiles.is_empty()
        || service_profiles
            .iter()
            .any(|profile| requested_profiles.contains(profile))
}

/// Evidence from executing a Deploy Plan: every operation completed, or the
/// completed prefix plus the failed operation and the unexecuted rest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[expect(
    clippy::large_enum_variant,
    reason = "Failed must own the named op and unexecuted rest; boxing would not change the states"
)]
pub enum DeployOutcome<E> {
    /// Every planned operation completed.
    Success { completed: Vec<DeployOperation> },
    /// Execution stopped at `failed`; `unexecuted` is the rest of the plan.
    Failed {
        completed: Vec<DeployOperation>,
        failed: FailedOperation<E>,
        unexecuted: Vec<DeployOperation>,
    },
}

/// Why a Deploy Operation did not complete.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum FailedOperation<E> {
    /// The named operation returned `error`.
    Operation {
        operation: DeployOperation,
        error: E,
    },
    /// Replacement started; health failed; `compensation` is what ran next.
    ReplacementHealth {
        operation: ReplacementOperation,
        error: E,
        compensation: ReplacementCompensation<E>,
    },
}

/// Compensation after a replacement health failure.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReplacementCompensation<E> {
    /// New container started first; `stop_new_container` is that stop attempt.
    StartFirst { stop_new_container: Result<(), E> },
    /// Old container stopped first; `restart_old_container` is that restart attempt.
    StopFirst {
        stop_new_container: Result<(), E>,
        restart_old_container: RestartAttempt<E>,
    },
}

/// Whether restarting the old container was attempted after stop-first replacement failure.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RestartAttempt<E> {
    /// Restart was not attempted.
    NotAttempted,
    /// Restart was attempted; `Ok(())` or the restart error.
    Attempted(Result<(), E>),
}

/// Replace one container with a newly resolved spec on the same Machine.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplacementOperation {
    /// Machine that hosts both containers.
    pub machine_id: MachineId,
    /// Container being replaced.
    pub old_container_id: ContainerId,
    /// Spec for the replacement container.
    pub spec: ResolvedServiceSpec,
    /// Skip waiting on container health after the replacement starts.
    pub skip_health_monitor: bool,
}

/// One step in a Deploy Plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DeployOperation {
    /// Create a Service Volume on `machine_id`.
    CreateVolume {
        machine_id: MachineId,
        volume: ServiceVolume,
    },
    /// Create and start a container from `spec`.
    RunContainer {
        machine_id: MachineId,
        spec: ResolvedServiceSpec,
        skip_health_monitor: bool,
    },
    /// Stop a running container.
    StopContainer {
        machine_id: MachineId,
        container_id: ContainerId,
    },
    /// Remove a stopped container.
    RemoveContainer {
        machine_id: MachineId,
        container_id: ContainerId,
    },
    /// Replace `old_container_id` with a new container from the replacement spec.
    ReplaceContainer(ReplacementOperation),
    /// Run the stop hook for a container.
    StopHook {
        machine_id: MachineId,
        container_id: ContainerId,
    },
    /// Run the start hook for a replacement, with prior hook containers to clean up.
    RunHook {
        machine_id: MachineId,
        spec: ResolvedServiceSpec,
        old_hook_containers: Vec<(MachineId, ContainerId)>,
    },
}

/// Observer-relative plan-plus-warnings offered for confirmation before one Deploy executes.
///
/// Live Observation shaped for a decision, not persisted state. Not a handle:
/// executing submits the same Deploy Intent and re-plans against a fresh snapshot.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeployPreview {
    /// Operations the current snapshot would execute.
    pub operations: Vec<DeployOperation>,
    /// Observer-relative warnings for this snapshot, including ingress DNS misses.
    pub warnings: Vec<DeployWarning>,
    /// Visible Services in the Project that Compose no longer declares.
    #[serde(default)]
    pub would_remove: Vec<QualifiedService>,
    /// Why pruning will not run. `None` still does not remove; this command never prunes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prune_refusal: Option<PruneRefusal>,
}

/// Why a loaded Compose Project is incomplete for reconciliation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComposePruneRefusal {
    /// Profiled Services were removed before planning.
    FilteredProfiles,
    /// The Project name was guessed from a directory while a non-default Compose file was named explicitly.
    GuessedProjectName,
}

impl From<ComposePruneRefusal> for PruneRefusal {
    fn from(reason: ComposePruneRefusal) -> Self {
        match reason {
            ComposePruneRefusal::FilteredProfiles => Self::FilteredProfiles,
            ComposePruneRefusal::GuessedProjectName => Self::GuessedProjectName,
        }
    }
}

/// Why a full reconciliation must not remove visible drift.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PruneRefusal {
    /// Required Container or Docker Volume evidence is missing from this Machine's visible fan-out.
    IncompleteSnapshot,
    /// The command named specific Services, so it is not a full reconciliation.
    SelectedServices,
    /// Profiled Services were removed before planning.
    FilteredProfiles,
    /// The Project name was guessed from a directory while a non-default Compose file was named explicitly.
    GuessedProjectName,
}

impl Display for PruneRefusal {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncompleteSnapshot => f.write_str(
                "Pruning is disabled because the Deploy Snapshot is incomplete relative to this Machine's current visible fan-out; required Container or Docker Volume evidence is missing. This is not Cluster completeness.",
            ),
            Self::SelectedServices => f.write_str(
                "Pruning is disabled because this command named specific Services, so it is not a full reconciliation.",
            ),
            Self::FilteredProfiles => f.write_str(
                "Pruning is disabled because profiled Services were removed before planning, so the loaded Compose Project is incomplete for reconciliation.",
            ),
            Self::GuessedProjectName => f.write_str(
                "Pruning is disabled because the Project name was guessed from a directory while a non-default Compose file was named explicitly.",
            ),
        }
    }
}

/// Kind of Machine observation that failed or was omitted while gathering a snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    Container,
    Volume,
}

impl Display for ObservationKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Container => f.write_str("container"),
            Self::Volume => f.write_str("volume"),
        }
    }
}

/// A warning attached to a Deploy Preview. Display matches the CLI warning body.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DeployWarning {
    /// Listing containers or volumes on `machine_id` returned `message`.
    ObservationFailed {
        kind: ObservationKind,
        machine_id: MachineId,
        message: String,
    },
    /// Listing containers or volumes on `machine_id` produced no terminal response.
    ObservationOmitted {
        kind: ObservationKind,
        machine_id: MachineId,
    },
    /// An Ingress Hostname misses this Cluster. The string is the CLI warning body.
    IngressHostname(String),
}

impl Display for DeployWarning {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ObservationFailed {
                kind,
                machine_id,
                message,
            } => write!(f, "{kind} observation failed on {machine_id}: {message}"),
            Self::ObservationOmitted { kind, machine_id } => {
                write!(f, "{kind} observation omitted {machine_id}")
            }
            Self::IngressHostname(message) => f.write_str(message),
        }
    }
}

/// Machine RPC invoked while executing one Deploy Operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MachineAction {
    CreateVolume,
    CreateContainer,
    StartContainer,
    InspectContainer,
    StopContainer,
    RemoveContainer,
}

/// Why health monitoring rejected a started container.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HealthFailure {
    Cancelled,
    TimedOut,
    Runtime(ContainerRuntimeObservation),
}

/// Why a pre-deploy hook container did not succeed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HookFailure {
    Cancelled { stop_error: Option<RpcError> },
    TimedOut { stop_error: Option<RpcError> },
    Exit(i64),
}

/// Error from executing one Deploy Operation.
#[derive(Clone, Debug, Error, PartialEq, Serialize, Deserialize)]
pub enum ExecutionError {
    #[error("{action:?} failed: {}", error.message)]
    Machine {
        action: MachineAction,
        error: RpcError,
    },
    #[error("container {container_id} failed health monitoring: {failure:?}")]
    Health {
        container_id: ContainerId,
        failure: HealthFailure,
    },
    #[error("hook container {container_id} failed: {failure:?}")]
    Hook {
        container_id: ContainerId,
        failure: HookFailure,
    },
}
