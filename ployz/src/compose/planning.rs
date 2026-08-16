use thiserror::Error;

use crate::deploy::{
    DeploySnapshot, PlanOptions, ProjectDeployPlan, ProjectPlanError, plan_services,
};

use super::{ComposeError, ComposeProject};

pub type ComposeDeployPlan = ProjectDeployPlan;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ComposePlanError {
    #[error(transparent)]
    Compose(#[from] ComposeError),
    #[error(transparent)]
    Plan(#[from] ProjectPlanError),
}

pub fn plan_compose_deploy(
    project: &ComposeProject,
    snapshot: &DeploySnapshot,
    options: PlanOptions,
) -> Result<ComposeDeployPlan, ComposePlanError> {
    let mut resolved = project.clone();
    resolved.resolve_secrets()?;
    Ok(plan_services(
        &resolved.dependency_order()?,
        snapshot,
        options,
    )?)
}
