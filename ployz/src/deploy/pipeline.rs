//! The Deploy pipeline: snapshot → preview → confirm.
//!
//! A Deploy Snapshot is gathered, a Deploy Preview is calculated, and
//! `confirm` executes those rows to a Deploy Outcome. This module does not
//! print, read stdin, or exit the process.

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::time::SystemTime;

use ployz_core::{
    DeployOperation, MachineFailure, MachineId, MachineObservation, PortPublication, ProjectName,
    RequestedServiceSpec, RpcError, ServiceMode, ServiceSelector, select_service,
};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::{
    compose::{BuildService, ComposeProject},
    connect::{Client, ConnectError},
    dns::{IngressDnsWarning, resolve_ingress_dns_warnings_for_ports},
    failure::Failure,
    image::PushError,
};

use super::{
    ComposePruneRefusal, DeployEvent, DeployIntent, DeployOutcome, DeployPreview, DeploySnapshot,
    DeployWarning, ExecutionError, IngressContext, ObservationKind, PlanError, PlanOptions,
    exec::execute_operation_sequence, planning, preview_deploy,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ReconciliationHints {
    pub requested_profiles: Vec<String>,
    pub compose_refusal: Option<ComposePruneRefusal>,
}

/// Snapshot or planning failure before a Deploy executes.
///
/// Execution failure is a [`DeployOutcome::Failed`], not this error.
#[derive(Debug, Error)]
pub enum DeployError {
    #[error(transparent)]
    Connect(#[from] ConnectError),
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error(transparent)]
    Project(#[from] crate::project::ProjectError),
}

impl Client {
    /// Calculate a Deploy Preview for a Deploy Intent without executing it.
    ///
    /// Gathers the observer-relative snapshot and reserved domain, calls
    /// [`preview_deploy`], then performs public-DNS warning I/O. Confirming
    /// executes these operations; it does not re-plan.
    ///
    /// # Errors
    ///
    /// Returns when snapshot gathering or planning fails.
    pub async fn preview(&mut self, intent: DeployIntent) -> Result<DeployPreview, DeployError> {
        let machines = self.machines().await?;
        let (snapshot, warnings) = gather_snapshot(self, machines).await?;
        preview_gathered(self, snapshot, warnings, &intent).await
    }

    /// Calculate a Project-removal preview. Confirming executes these operations.
    ///
    /// Reserved names reuse [`crate::project::refuse_reserved`]. Incomplete
    /// snapshots reuse [`DeployIntent::prune_refusal`].
    ///
    /// # Errors
    ///
    /// Returns when the Project is reserved or snapshot gathering or planning fails.
    pub async fn preview_project_removal(
        &mut self,
        project: &ProjectName,
        volumes: super::VolumeFate,
    ) -> Result<DeployPreview, DeployError> {
        crate::project::refuse_reserved(project)?;
        let machines = self.machines().await?;
        let (snapshot, warnings) = gather_snapshot(self, machines).await?;
        let preview = planning::plan_project_removal(project, &snapshot, volumes)?;
        Ok(attach_leading_warnings(preview, warnings))
    }

    /// Execute the operations on this Deploy Preview. Does not re-plan.
    ///
    /// Progress events are sent on `progress` when provided. The first event is
    /// every row `pending` before any Machine RPC. Execution failure is a
    /// [`DeployOutcome::Failed`], not this error.
    pub async fn confirm(
        &self,
        preview: &DeployPreview,
        cancellation: &CancellationToken,
        progress: Option<tokio::sync::mpsc::UnboundedSender<DeployEvent>>,
    ) -> DeployOutcome<ExecutionError> {
        execute_operation_sequence(
            preview.operations.clone(),
            self,
            cancellation,
            progress,
            &preview.project_name,
        )
        .await
    }

    /// Preview, auto-confirm, and return the Deploy Outcome.
    ///
    /// # Errors
    ///
    /// Returns when snapshot gathering or planning fails
    /// before execution starts.
    pub async fn run(
        &mut self,
        intent: DeployIntent,
        cancellation: &CancellationToken,
        progress: Option<tokio::sync::mpsc::UnboundedSender<DeployEvent>>,
    ) -> Result<DeployOutcome<ExecutionError>, DeployError> {
        let preview = self.preview(intent).await?;
        Ok(self.confirm(&preview, cancellation, progress).await)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PushOutcome {
    pub pushed: Vec<PushedImage>,
    pub failures: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PushedImage {
    pub image: String,
    pub machine_id: MachineId,
}

impl From<IngressDnsWarning> for DeployWarning {
    fn from(warning: IngressDnsWarning) -> Self {
        Self::IngressHostname(warning.to_string())
    }
}

pub(super) async fn push_project_images(
    client: &mut Client,
    builds: &[BuildService],
    machines: &[MachineObservation],
) -> Result<PushOutcome, Failure> {
    let mut pushed = Vec::new();
    let mut failures = Vec::new();
    for service in builds {
        match push_image(client, service, machines).await {
            Ok((images, service_failures)) => {
                pushed.extend(images);
                failures.extend(service_failures);
            }
            Err(error) => failures.push(format!("{}: {error}", service.image)),
        }
    }
    Ok(PushOutcome { pushed, failures })
}

pub(super) async fn plan_project(
    client: &mut Client,
    project: &mut ComposeProject,
    machines: Vec<MachineObservation>,
    options: PlanOptions,
    project_name: &ProjectName,
    hints: ReconciliationHints,
) -> Result<DeployPreview, Failure> {
    project.resolve_secrets()?;
    let (snapshot, warnings) = gather_snapshot(client, machines).await?;
    super::reject_missing_external_volumes(project, &snapshot)?;
    let intent = DeployIntent::from_named_specs(
        project_name.clone(),
        &project.services,
        &project.dependencies,
        options,
    )
    .with_service_profiles(project.service_profiles())
    .with_requested_profiles(hints.requested_profiles)
    .with_compose_refusal(hints.compose_refusal);
    Ok(preview_gathered(client, snapshot, warnings, &intent).await?)
}

pub(super) async fn plan_scale(
    client: &mut Client,
    selector: &ServiceSelector,
    replicas: NonZeroU32,
    options: PlanOptions,
) -> Result<(DeployPreview, ProjectName), Failure> {
    let machines = list_machines(client).await?;
    let (snapshot, warnings) = gather_snapshot(client, machines).await?;
    let choice = choose_scale_spec(&snapshot, selector, replicas)?;
    let Some(requested) = choice.requested else {
        return Ok((
            DeployPreview::new(Vec::new(), warnings, choice.project_name.clone()),
            choice.project_name,
        ));
    };
    let intent = DeployIntent::apply_one(choice.project_name.clone(), requested, options);
    Ok((
        preview_gathered(client, snapshot, warnings, &intent).await?,
        choice.project_name,
    ))
}

async fn preview_gathered(
    client: &mut Client,
    snapshot: DeploySnapshot,
    mut warnings: Vec<DeployWarning>,
    intent: &DeployIntent,
) -> Result<DeployPreview, DeployError> {
    let domain = if intent.target.iter().any(needs_ingress_expansion) {
        client.domain_if_reserved().await?
    } else {
        None
    };
    let preview = preview_deploy(
        intent,
        &snapshot,
        IngressContext {
            cluster_domain: domain.as_deref(),
        },
    )?;
    warnings.extend(hostname_warnings(&preview, &snapshot.machines).await);
    Ok(attach_leading_warnings(preview, warnings))
}

fn attach_leading_warnings(
    mut preview: DeployPreview,
    mut warnings: Vec<DeployWarning>,
) -> DeployPreview {
    warnings.extend(std::mem::take(&mut preview.warnings));
    preview.warnings = warnings;
    preview
}

pub(crate) fn plan_options(force_recreate: bool, skip_health_monitor: bool) -> PlanOptions {
    PlanOptions {
        force_recreate,
        skip_health_monitor,
        placement_seed: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos() as u64),
        ..PlanOptions::default()
    }
}

#[derive(Debug)]
struct ScaleSpec {
    project_name: ProjectName,
    requested: Option<RequestedServiceSpec>,
}

fn choose_scale_spec(
    snapshot: &DeploySnapshot,
    selector: &ServiceSelector,
    replicas: NonZeroU32,
) -> Result<ScaleSpec, Failure> {
    let services = ployz_core::derive_services(snapshot.containers.iter().cloned());
    let service = select_service(&services, selector)?;
    let observed_container = service
        .containers
        .first()
        .ok_or_else(|| Failure::usage("cannot scale a service without regular containers"))?
        .as_observation();
    match observed_container.resolved_spec.mode {
        ServiceMode::Replicated { .. } => {}
        ServiceMode::Global => return Err(Failure::usage("global services cannot be scaled")),
    }
    let project_name = service.identity.project.clone();
    if usize::try_from(replicas.get()) == Ok(service.containers.len()) {
        return Ok(ScaleSpec {
            project_name,
            requested: None,
        });
    }
    // TODO(UT-046): mixed historical specs use one observed regular container; there is no chooser.
    let mut requested = observed_container.resolved_spec.to_requested();
    requested.mode = ServiceMode::Replicated { replicas };
    Ok(ScaleSpec {
        project_name,
        requested: Some(requested),
    })
}

pub(super) async fn list_machines(client: &mut Client) -> Result<Vec<MachineObservation>, Failure> {
    Ok(client.machines().await?)
}

async fn gather_snapshot(
    client: &mut Client,
    machines: Vec<MachineObservation>,
) -> Result<(DeploySnapshot, Vec<DeployWarning>), DeployError> {
    let snapshot = client.deploy_snapshot(machines).await?;
    let mut warnings = observation_warnings(
        ObservationKind::Container,
        &snapshot.container_failures,
        &snapshot.container_omissions,
    );
    warnings.extend(observation_warnings(
        ObservationKind::Volume,
        &snapshot.volume_failures,
        &snapshot.volume_omissions,
    ));
    Ok((snapshot, warnings))
}

fn observation_warnings(
    kind: ObservationKind,
    failures: &[MachineFailure<RpcError>],
    omissions: &[MachineId],
) -> Vec<DeployWarning> {
    failures
        .iter()
        .map(|failure| DeployWarning::ObservationFailed {
            kind,
            machine_id: failure.machine_id,
            message: failure.error.message.clone(),
        })
        .chain(
            omissions
                .iter()
                .map(|machine| DeployWarning::ObservationOmitted {
                    kind,
                    machine_id: *machine,
                }),
        )
        .collect()
}

async fn hostname_warnings(
    preview: &DeployPreview,
    machines: &[MachineObservation],
) -> Vec<DeployWarning> {
    resolve_ingress_dns_warnings_for_ports(
        preview_ports(preview),
        &machine_public_addresses(machines),
    )
    .await
    .into_iter()
    .map(DeployWarning::from)
    .collect()
}

fn preview_ports(preview: &DeployPreview) -> impl Iterator<Item = &PortPublication> {
    preview
        .operations
        .iter()
        .flat_map(|row| match &row.operation {
            DeployOperation::RunContainer { spec, .. } | DeployOperation::RunHook { spec, .. } => {
                spec.ports.as_slice()
            }
            DeployOperation::ReplaceContainer(replacement) => replacement.spec.ports.as_slice(),
            DeployOperation::CreateVolume { .. }
            | DeployOperation::StopContainer { .. }
            | DeployOperation::RemoveContainer { .. }
            | DeployOperation::StopHook { .. }
            | DeployOperation::RemoveVolume { .. } => &[],
        })
}

fn machine_public_addresses(machines: &[MachineObservation]) -> Vec<IpAddr> {
    machines
        .iter()
        .filter_map(|machine| machine.machine.public_ip)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn needs_ingress_expansion(requested: &RequestedServiceSpec) -> bool {
    requested
        .ports
        .iter()
        .any(|port| matches!(port, PortPublication::Ingress { .. }))
}

async fn push_image(
    client: &mut Client,
    service: &BuildService,
    machines: &[MachineObservation],
) -> Result<(Vec<PushedImage>, Vec<String>), PushError> {
    let targets = service
        .machines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let result =
        crate::image::push_using_machines(client, &service.image, None, &targets, machines).await?;
    let pushed = result
        .successes
        .iter()
        .map(|success| PushedImage {
            image: service.image.clone(),
            machine_id: success.machine_id,
        })
        .collect();
    let failures = result
        .failures
        .into_iter()
        .map(|failure| {
            format!(
                "{} on {}: {}",
                service.image, failure.machine_id, failure.error
            )
        })
        .chain(
            result
                .omissions
                .into_iter()
                .map(|machine| format!("{} on {machine}: no terminal response", service.image)),
        )
        .collect();
    Ok((pushed, failures))
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
