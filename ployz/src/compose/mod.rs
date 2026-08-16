use thiserror::Error;

use crate::deploy::{
    DeploySnapshot, PlanOptions, ProjectDeployPlan, ProjectPlanError, plan_services,
};

mod build;
mod configs;
mod convert;
mod extensions;
mod image;
mod loader;
mod model;
mod mounts;
mod ports;
mod secrets;

pub use build::{BuildOptions, BuildService, execute_build, plan_build};
pub use convert::parse_normalized;
pub use loader::{LoadOptions, load_project};
pub use model::{BuildSpec, ComposeError, ComposeProject};
pub(crate) use ports::parse_extension_port;

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

pub(crate) fn parse_bytes(value: &str) -> Option<u64> {
    convert::bytes_u64(&serde_norway::Value::String(value.into()))
}
