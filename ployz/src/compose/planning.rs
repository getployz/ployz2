use std::collections::BTreeMap;

use ployz_core::{DockerVolumeName, ServiceId, ServiceMode, VolumeSource};
use thiserror::Error;

use crate::deploy::{
    DeployOperation, DeployPlan, DeploySnapshot, PlanError, PlanOptions, claim_shared_volumes,
    plan_deploy,
};

use super::{ComposeError, ComposeProject};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposeDeployPlan {
    pub volume_operations: Vec<DeployOperation>,
    pub service_plans: Vec<DeployPlan>,
}

impl ComposeDeployPlan {
    #[must_use]
    pub fn operations(&self) -> Vec<&DeployOperation> {
        self.volume_operations
            .iter()
            .chain(
                self.service_plans
                    .iter()
                    .flat_map(|plan| plan.service_operations.iter()),
            )
            .collect()
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ComposePlanError {
    #[error(transparent)]
    Compose(#[from] ComposeError),
    #[error("plan service '{service}': {source}")]
    Service { service: String, source: PlanError },
    #[error(
        "Docker Volume {name} cannot be shared by global service '{global}' and replicated service '{replicated}'"
    )]
    MixedVolumeModes {
        name: DockerVolumeName,
        global: String,
        replicated: String,
    },
}

pub fn plan_compose_deploy(
    project: &ComposeProject,
    snapshot: &DeploySnapshot,
    options: PlanOptions,
) -> Result<ComposeDeployPlan, ComposePlanError> {
    let resolved_owned;
    let project = if needs_secret_resolution(project) {
        let mut resolved = project.clone();
        resolved.resolve_secrets()?;
        resolved_owned = resolved;
        &resolved_owned
    } else {
        project
    };
    let volume_uses = named_volume_uses(project);
    reject_mixed_volume_modes(&volume_uses)?;
    let ordered = project.dependency_order()?;
    let mut claims =
        claim_shared_volumes(ordered.iter().copied(), snapshot, options).map_err(|source| {
            ComposePlanError::Service {
                service: ordered
                    .first()
                    .map(|spec| spec.name.to_string())
                    .unwrap_or_default(),
                source,
            }
        })?;
    let mut volume_operations = std::mem::take(&mut claims.volume_operations);
    let mut service_plans = Vec::new();
    for requested in ordered {
        let service = requested.name.to_string();
        let plan = plan_deploy(requested, snapshot, ServiceId::random(), options, &claims)
            .map_err(|source| ComposePlanError::Service {
                service: service.clone(),
                source,
            })?;
        volume_operations.extend(plan.volume_operations);
        // TODO(UT-088): depends_on conditions are not represented as first operations.
        service_plans.push(DeployPlan::new(
            plan.service_id,
            plan.is_new_service,
            Vec::new(),
            plan.service_operations,
        ));
    }
    Ok(ComposeDeployPlan {
        volume_operations,
        service_plans,
    })
}

fn needs_secret_resolution(project: &ComposeProject) -> bool {
    project.services.values().any(|spec| {
        spec.container
            .environment
            .values()
            .any(|value| value.starts_with("secret://"))
    })
}

#[derive(Clone, Copy)]
struct NamedVolumeUse<'a> {
    service_name: &'a str,
    global: bool,
}

fn named_volume_uses(
    project: &ComposeProject,
) -> BTreeMap<DockerVolumeName, Vec<NamedVolumeUse<'_>>> {
    let mut uses = BTreeMap::<DockerVolumeName, Vec<NamedVolumeUse<'_>>>::new();
    for (service_name, service) in &project.services {
        for mount in &service.mounts {
            let Some(volume) = service
                .volumes
                .iter()
                .find(|volume| volume.reference == mount.volume)
            else {
                continue;
            };
            let VolumeSource::Named { name, .. } = &volume.source else {
                continue;
            };
            let uses = uses.entry(name.clone()).or_default();
            if !uses
                .iter()
                .any(|volume_use| volume_use.service_name == service_name)
            {
                uses.push(NamedVolumeUse {
                    service_name,
                    global: matches!(service.mode, ServiceMode::Global),
                });
            }
        }
    }
    uses
}

fn reject_mixed_volume_modes(
    volume_uses: &BTreeMap<DockerVolumeName, Vec<NamedVolumeUse<'_>>>,
) -> Result<(), ComposePlanError> {
    for (name, uses) in volume_uses {
        if let (Some(global), Some(replicated)) = (
            uses.iter().find(|volume_use| volume_use.global),
            uses.iter().find(|volume_use| !volume_use.global),
        ) {
            return Err(ComposePlanError::MixedVolumeModes {
                name: name.clone(),
                global: global.service_name.into(),
                replicated: replicated.service_name.into(),
            });
        }
    }
    Ok(())
}
