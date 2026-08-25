use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::ValueError;

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
