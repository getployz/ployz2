//! Cloud preparation: builder eligibility, Builds, planning, and image delivery.
use crate::connect::ConnectError;
use ployz_core::{
    DescribeContractRequest, MachineId, MachineObservation, MachineTarget, RpcError, RpcErrorCode,
    op,
};
use std::time::Duration;

/// Selected builder, why it was chosen, and rejected observations, for caller-owned presentation.
#[derive(serde::Serialize)]
pub struct SelectedBuilder {
    pub machine: ployz_core::Machine,
    pub reason: BuilderReason,
    pub rejections: Vec<String>,
}

/// Why a Server was chosen to build: evidence, never a prediction.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BuilderReason {
    /// It is the Server named in the Service's latest Build Receipt.
    HadCache,
    /// No Server held the Service's build cache; builds spread across Servers.
    Spread,
    /// The cache holder is offline or no longer builds; builds spread across Servers.
    CacheHolderUnavailable { holder: String },
}

/// Where a build should go, from what the caller already knows.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuildPreference {
    /// The Server named in the Service's latest Build Receipt.
    pub cache_holder: Option<MachineId>,
    /// This build's position among its attempt's builds.
    pub spread: usize,
}

fn may_build(observed: &MachineObservation) -> bool {
    observed.membership.invites_rpc() && observed.machine.accepts_builds
}

/// Rank the cache holder first, then Servers that accept Builds, rotated by
/// `spread` so an attempt's builds start on different Servers. The rest follow
/// only so their rejections are reported. Load from other attempts is not
/// considered; it waits in each Server's queue.
fn rank(visible: &[MachineObservation], preference: BuildPreference) -> Vec<&MachineObservation> {
    let mut ranked = visible.iter().collect::<Vec<_>>();
    ranked.sort_by_key(|observed| (!may_build(observed), observed.machine.id));
    let eligible = ranked.iter().filter(|observed| may_build(observed)).count();
    if let Some(eligible) = ranked.get_mut(..eligible).filter(|slice| !slice.is_empty()) {
        let spread = preference.spread % eligible.len();
        eligible.rotate_left(spread);
    }
    if let Some(at) = ranked
        .iter()
        .position(|observed| Some(observed.machine.id) == preference.cache_holder)
    {
        let holder = ranked.remove(at);
        ranked.insert(0, holder);
    }
    ranked
}

fn builder_reason(
    visible: &[MachineObservation],
    preference: BuildPreference,
    selected: MachineId,
) -> BuilderReason {
    match preference.cache_holder {
        None => BuilderReason::Spread,
        Some(holder) if holder == selected => BuilderReason::HadCache,
        Some(holder) => BuilderReason::CacheHolderUnavailable {
            holder: visible
                .iter()
                .find(|observed| observed.machine.id == holder)
                .map_or_else(
                    || holder.to_string(),
                    |observed| observed.machine.name.to_string(),
                ),
        },
    }
}

/// Select the first capable observed Machine in [`rank`] order.
/// # Errors
/// Returns exhausted eligibility evidence.
pub async fn select_build_machine(
    client: &mut crate::connect::Client,
    targets: &[ployz_build::Target],
    visible: &[MachineObservation],
    preference: BuildPreference,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<SelectedBuilder, ConnectError> {
    let candidates = rank(visible, preference);
    let mut reasons = Vec::new();
    let mut rejected = std::collections::BTreeMap::<&str, usize>::new();
    for observed in candidates {
        if cancellation.is_cancelled() {
            reasons.push("Build selection cancelled".into());
            break;
        }
        let machine = &observed.machine;
        let category = if !observed.membership.invites_rpc() {
            "membership unavailable"
        } else if !machine.accepts_builds {
            "builds disabled"
        } else {
            "capability unverified"
        };
        let reason = if !observed.membership.invites_rpc() {
            format!("membership is {:?}", observed.membership)
        } else if !machine.accepts_builds {
            "does not accept Builds".into()
        } else {
            match client
                .invoke::<op::DescribeContract>(
                    DescribeContractRequest {},
                    &MachineTarget::from(&machine.id),
                    Some(Duration::from_secs(5)),
                )
                .await
            {
                Ok(contract) if contract.machine_id != machine.id => format!(
                    "contract identifies a different Machine ({})",
                    contract.machine_id
                ),
                Ok(contract) if !contract.supports(ployz_core::BUILD_CAPABILITY) => {
                    "does not support remote Builds".into()
                }
                Ok(_) => match client
                    .check_build_capabilities(machine.id, targets, cancellation)
                    .await
                {
                    Ok(()) => {
                        return Ok(SelectedBuilder {
                            machine: machine.clone(),
                            reason: builder_reason(visible, preference, machine.id),
                            rejections: reasons,
                        });
                    }
                    Err(error) => format!("Build capability could not be verified: {error}"),
                },
                Err(error) => format!("Build capability could not be verified: {error}"),
            }
        };
        *rejected.entry(category).or_default() += 1;
        reasons.push(format!(
            "Machine {} ({}): {reason}",
            machine.name, machine.id
        ));
    }
    Err(ConnectError::Remote(RpcError {
        code: RpcErrorCode::Unsupported,
        message: format!(
            "no eligible Build Machine: {}",
            if reasons.is_empty() {
                "no Machines observed".into()
            } else {
                reasons.join("; ")
            }
        ),
        details: serde_json::json!({"rejections": rejected}),
    }))
}

use crate::{
    build::{BuiltService, CapturedBuild},
    connect::Client,
    deploy::{DeployError, DeployPlan},
};
use ployz_core::DeployIntent;
use tokio_util::sync::CancellationToken;

/// Preparation failures never imply application confirmation.
#[derive(Debug, thiserror::Error)]
pub enum PreparationError {
    #[error("{0}. No Service, hook, or volume change was attempted.")]
    Build(#[from] crate::build::Error),
    #[error(transparent)]
    Connect(#[from] ConnectError),
    #[error("Build selection failed: {0}")]
    Selection(ConnectError),
    #[error(transparent)]
    Plan(#[from] DeployError),
    #[error("{0}")]
    Delivery(String),
    #[error("preparation cancelled; no application changes attempted")]
    Cancelled,
}

impl PreparationError {
    /// Cancellation reached this Build before any source left the client: it
    /// was stopped while reading, selecting, or waiting in the Machine's queue.
    pub(super) fn cancelled_before_upload(&self) -> bool {
        use crate::build::{Error, RemoteBuildFailure};
        use ployz_build::Stage;
        match self {
            Self::Cancelled => true,
            Self::Build(Error::RemoteBuild { outcome }) => matches!(
                **outcome,
                RemoteBuildFailure::Failed {
                    stage: Stage::Queued | Stage::Admission,
                    ..
                }
            ),
            Self::Build(_)
            | Self::Connect(_)
            | Self::Selection(_)
            | Self::Plan(_)
            | Self::Delivery(_) => false,
        }
    }
}

/// Caller-rendered preparation progress; no terminal or process ownership.
#[derive(serde::Serialize)]
pub enum Progress {
    Platforms(Vec<String>),
    Selected(Box<SelectedBuilder>),
    Build(ployz_build::Progress),
    Transfer,
    Delivered {
        image: String,
        machine_id: ployz_core::MachineId,
    },
}

/// Prepared plan with image response streams retained until its owner is dropped.
pub struct Prepared {
    pub plan: DeployPlan,
    builds: Vec<BuiltService>,
}

impl Prepared {
    /// Move plan and retained image owners together into a prepared/running handle.
    #[must_use]
    pub fn into_parts(self) -> (DeployPlan, Vec<BuiltService>) {
        (self.plan, self.builds)
    }
}

/// Build captured source, bind verified images, plan fresh destinations and deliver.
/// # Errors
/// Returns typed Build evidence, eligibility, planning, cancellation or delivery failures.
pub async fn prepare(
    client: &mut Client,
    mut intent: DeployIntent,
    build: CapturedBuild,
    reusable: &[BuiltService],
    preference: BuildPreference,
    cancellation: &CancellationToken,
    progress: impl Fn(Progress),
) -> Result<Prepared, PreparationError> {
    let builds = build_images(
        client,
        &intent,
        build,
        reusable,
        preference,
        cancellation,
        &progress,
    )
    .await?;
    crate::build::bind(&mut intent, &builds)?;
    let machines = read(cancellation, async { Ok(client.machines().await?) }).await?;
    let plan = read(cancellation, async {
        Ok(crate::deploy::pipeline::plan_project(client, &intent, machines.clone()).await?)
    })
    .await?;
    if !builds.is_empty() {
        progress(Progress::Transfer);
    }
    let outcome = crate::deploy::pipeline::push_project_images(
        client,
        &builds,
        &machines,
        &plan,
        cancellation,
    )
    .await?;
    for image in outcome.pushed {
        progress(Progress::Delivered {
            image: image.image,
            machine_id: image.machine_id,
        });
    }
    if cancellation.is_cancelled() {
        return Err(PreparationError::Cancelled);
    }
    if !outcome.failures.is_empty() {
        return Err(PreparationError::Delivery(format!(
            "image push failed: {}",
            outcome.failures.join("; ")
        )));
    }
    Ok(Prepared { plan, builds })
}

/// Reuse still-available images, then build the remaining targets on one
/// Build Machine ranked by `preference`. `prepare` and the one-Service `Session::build` share this.
/// # Errors
/// Returns typed Build evidence across every target, eligibility, or cancellation.
pub(super) async fn build_images(
    client: &mut Client,
    intent: &DeployIntent,
    mut build: CapturedBuild,
    reusable: &[BuiltService],
    preference: BuildPreference,
    cancellation: &CancellationToken,
    progress: &impl Fn(Progress),
) -> Result<Vec<BuiltService>, PreparationError> {
    if build.targets().next().is_none() {
        return Ok(Vec::new());
    }
    let machines = observe_machines(client, intent, cancellation).await?;
    build.cover_machines(intent, &machines)?;
    let mut builds = build
        .reuse_images(client, intent, &machines, reusable, cancellation)
        .await;
    let targets = build.to_targets();
    let platforms = targets
        .iter()
        .flat_map(|target| target.platforms.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    progress(Progress::Platforms(platforms.into_iter().collect()));
    if targets.is_empty() {
        return Ok(builds);
    }
    let selected = read(cancellation, async {
        select_build_machine(client, &targets, &machines, preference, cancellation)
            .await
            .map_err(PreparationError::Selection)
    })
    .await?;
    let id = selected.machine.id;
    progress(Progress::Selected(Box::new(selected)));
    // Await terminal evidence and cleanup; cancelling this future would erase Unknown.
    builds.extend(
        build
            .execute_remote_images(client, id, cancellation.clone(), |event| {
                progress(Progress::Build(event))
            })
            .await?,
    );
    Ok(builds)
}

/// The Machines and, when a Service mounts provisioned volumes, their storage:
/// what placement, and so the Build Platform Requirement, is read from.
pub(super) async fn observe_machines(
    client: &mut Client,
    intent: &DeployIntent,
    cancellation: &CancellationToken,
) -> Result<Vec<MachineObservation>, PreparationError> {
    let mut machines = read(cancellation, async { Ok(client.machines().await?) }).await?;
    let applied = intent.applied_names();
    if intent.target.iter().any(|spec| {
        applied.contains(&spec.name) && spec.volume_graph().has_mounted_provisioned_volume()
    }) {
        read(cancellation, async {
            client.observe_machine_storage(&mut machines).await;
            Ok(())
        })
        .await?;
    }
    Ok(machines)
}

async fn read<T>(
    cancel: &CancellationToken,
    work: impl std::future::Future<Output = Result<T, PreparationError>>,
) -> Result<T, PreparationError> {
    tokio::select! { biased; () = cancel.cancelled() => Err(PreparationError::Cancelled), result = work => result }
}
