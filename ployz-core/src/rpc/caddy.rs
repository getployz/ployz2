use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::QualifiedService;

/// Requests the active Caddyfile.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GetCaddyConfigRequest {}

/// A Service's state in a candidate Caddy configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CaddyServiceConfig {
    /// The Service remains deployed, with an optional custom Caddy fragment.
    Present(Option<String>),
    /// The Service is removed by the deployment.
    Removed,
}

/// The Service states used to build and validate a candidate Caddy configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PreflightCaddyConfigRequest {
    /// Final deployment state for every Service changed or removed by the plan.
    pub services: BTreeMap<QualifiedService, CaddyServiceConfig>,
}

/// Confirms that the candidate Caddy configuration adapted successfully.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CaddyConfigPreflighted {}

/// The active Caddyfile.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CaddyConfig {
    /// Rendered Caddyfile source.
    pub caddyfile: String,
}
