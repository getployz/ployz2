//! Reserved Ingress Proxy container policy at the Machine trust boundary.

use ployz_core::{
    ContainerKind, IngressProxyBackend, ProjectName, QualifiedService, ResolvedServiceSpec,
};

use super::{LocalMachine, LocalMachineError};
use crate::docker::NetworkAttachment;

impl LocalMachine {
    /// Refuse reserved Ingress Proxy work without its replicated authority.
    ///
    /// # Errors
    ///
    /// Returns when the Cluster backend is absent, invalid, or differs from
    /// the reserved Service's concrete wiring.
    pub(crate) async fn service_network(
        &self,
        kind: ContainerKind,
        project: &ProjectName,
        spec: &ResolvedServiceSpec,
    ) -> Result<NetworkAttachment, LocalMachineError> {
        if QualifiedService::new(project.clone(), spec.name.clone())
            == QualifiedService::system_ingress()
        {
            let backend = ployz_core::ingress_proxy_backend(spec)?;
            self.replicated()?
                .require_ingress_proxy_backend(backend)
                .await
                .map_err(LocalMachineError::IngressProxyBackend)?;
            return Ok(match backend {
                IngressProxyBackend::Caddy => NetworkAttachment::Bridge,
                IngressProxyBackend::Zentinel
                    if matches!(kind, ContainerKind::ServiceContainer) =>
                {
                    NetworkAttachment::Host
                }
                IngressProxyBackend::Zentinel => NetworkAttachment::Bridge,
            });
        }
        Ok(NetworkAttachment::Bridge)
    }
}
