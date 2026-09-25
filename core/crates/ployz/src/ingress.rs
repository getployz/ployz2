//! Ingress Proxy identity and deployment boundaries.

use ployz_core::{
    ContainerObservation, IngressProxyFragment, PlacementConstraint, QualifiedService,
    RequestedServiceSpec, caddy_service_spec,
};

mod caddy;
pub use caddy::IngressImageError;

/// Build the Caddy ingress Service Spec.
///
/// # Errors
///
/// Returns when the Caddy image cannot be discovered.
pub async fn service_spec(
    image: Option<String>,
    constraints: std::collections::BTreeSet<PlacementConstraint>,
    fragment: Option<IngressProxyFragment>,
) -> Result<RequestedServiceSpec, IngressImageError> {
    let image = match image {
        Some(image) => image,
        None => caddy::latest_image().await?,
    };
    Ok(caddy_service_spec(image, constraints, fragment))
}

/// True when this observation is the reserved Ingress Proxy Service.
#[must_use]
pub fn is_system_ingress(observation: &ContainerObservation) -> bool {
    observation.identity() == QualifiedService::system_ingress()
}
