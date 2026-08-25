//! Ingress Proxy identity and deployment boundaries.

use ployz_core::{ContainerObservation, QualifiedService};

mod caddy;

pub use caddy::{IngressImageError, latest_image, newest_existing_settings, service_spec};

/// Shared host data root owned by the Ingress Proxy responsibility.
pub const DATA_PATH: &str = "/var/lib/ployz/ingress";
/// Shared host runtime root owned by the Ingress Proxy responsibility.
pub const RUNTIME_PATH: &str = "/run/ployz/ingress";

/// True when this observation is the reserved Ingress Proxy Service.
#[must_use]
pub fn is_system_ingress(observation: &ContainerObservation) -> bool {
    observation.identity() == QualifiedService::system_ingress()
}
