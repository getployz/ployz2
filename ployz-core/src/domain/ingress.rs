use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{
    HostBind, IngressProxyFragment, PortPublication, RequestedServiceSpec, ResolvedServiceSpec,
    ServiceContainerSpec, TransportProtocol, ValueError,
};

/// Exact Caddy command for the reserved Ingress Proxy Service.
pub const CADDY_INGRESS_COMMAND: [&str; 4] = ["caddy", "run", "-c", "/config/caddy/Caddyfile"];
/// Caddy's loopback administration socket setting.
pub const CADDY_INGRESS_ADMIN: &str = "unix//run/ingress/caddy/admin.sock";
/// Exact Zentinel command for the reserved Ingress Proxy Service.
pub const ZENTINEL_INGRESS_COMMAND: [&str; 2] = ["-c", "/config/zentinel.kdl"];
/// The only Linux capability granted to Zentinel.
pub const ZENTINEL_INGRESS_CAPABILITY: &str = "NET_BIND_SERVICE";

/// Invalid or mixed concrete wiring for the reserved Ingress Proxy Service.
#[derive(Debug, thiserror::Error)]
#[error("reserved Ingress Proxy Service has missing, unknown, or mixed backend wiring")]
pub struct IngressProxyServiceSpecError;

/// Replicated key holding the Cluster's immutable Ingress Proxy Backend.
pub const INGRESS_PROXY_BACKEND_CLUSTER_KEY: &str = "ingress_proxy_backend";

/// Concrete Ingress Proxy implementation selected when a Cluster is founded.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressProxyBackend {
    Caddy,
    Zentinel,
}

impl IngressProxyBackend {
    /// Parse a replicated Ingress Proxy Backend value.
    ///
    /// # Errors
    ///
    /// Returns [`ValueError`] unless `value` is `caddy` or `zentinel`.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, ValueError> {
        let value = value.as_ref();
        match value {
            "caddy" => Ok(Self::Caddy),
            "zentinel" => Ok(Self::Zentinel),
            _ => Err(ValueError::new(
                "Ingress Proxy Backend",
                value,
                "caddy or zentinel",
            )),
        }
    }

    /// Stable replicated spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Caddy => "caddy",
            Self::Zentinel => "zentinel",
        }
    }
}

/// Recover the backend encoded by one concrete reserved-service spec.
///
/// # Errors
///
/// Returns [`IngressProxyServiceSpecError`] unless the spec has one exact
/// supported backend topology.
pub fn ingress_proxy_backend(
    spec: &ResolvedServiceSpec,
) -> Result<IngressProxyBackend, IngressProxyServiceSpecError> {
    ingress_proxy_backend_from_parts(
        &spec.container,
        &spec.ports,
        spec.ingress_proxy_fragment.as_ref(),
    )
}

/// Recover the backend encoded by one concrete requested reserved-service spec.
///
/// # Errors
///
/// Returns [`IngressProxyServiceSpecError`] unless the spec has one exact
/// supported backend topology.
pub fn requested_ingress_proxy_backend(
    spec: &RequestedServiceSpec,
) -> Result<IngressProxyBackend, IngressProxyServiceSpecError> {
    ingress_proxy_backend_from_parts(
        &spec.container,
        &spec.ports,
        spec.ingress_proxy_fragment.as_ref(),
    )
}

fn ingress_proxy_backend_from_parts(
    container: &ServiceContainerSpec,
    ports: &[PortPublication],
    ingress_proxy_fragment: Option<&IngressProxyFragment>,
) -> Result<IngressProxyBackend, IngressProxyServiceSpecError> {
    let command_is = |expected: &[&str]| {
        container
            .command
            .iter()
            .map(String::as_str)
            .eq(expected.iter().copied())
    };
    let caddy_ports = [
        (80, TransportProtocol::Tcp),
        (443, TransportProtocol::Tcp),
        (443, TransportProtocol::Udp),
    ];
    let is_caddy = command_is(&CADDY_INGRESS_COMMAND)
        && container.environment.len() == 1
        && container
            .environment
            .get("CADDY_ADMIN")
            .is_some_and(|admin| admin == CADDY_INGRESS_ADMIN)
        && container.cap_add.is_empty()
        && container.cap_drop.is_empty()
        && !container.privileged
        && ports.len() == caddy_ports.len()
        && ports.iter().zip(caddy_ports).all(|(port, expected)| {
            matches!(
                port,
                PortPublication::Host {
                    bind: HostBind::All,
                    published_port,
                    container_port,
                    transport_protocol,
                } if published_port.get() == expected.0
                    && container_port.get() == expected.0
                    && *transport_protocol == expected.1
            )
        });
    if is_caddy {
        return Ok(IngressProxyBackend::Caddy);
    }

    let is_zentinel = command_is(&ZENTINEL_INGRESS_COMMAND)
        && container.environment.is_empty()
        && container.cap_add == [ZENTINEL_INGRESS_CAPABILITY]
        && container.cap_drop == ["ALL"]
        && !container.privileged
        && ports.is_empty()
        && ingress_proxy_fragment.is_none();
    if is_zentinel {
        return Ok(IngressProxyBackend::Zentinel);
    }
    Err(IngressProxyServiceSpecError)
}

impl fmt::Display for IngressProxyBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for IngressProxyBackend {
    type Err = ValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}
