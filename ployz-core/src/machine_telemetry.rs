//! Fresh operator telemetry from one Machine.

use serde::{Deserialize, Deserializer, Serialize, de};

/// Consistent usable, attached, and free endpoint counts for the Ployz bridge.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct BridgeEndpointCapacity {
    #[serde(rename = "bridge_usable_endpoints")]
    usable_endpoints: u64,
    #[serde(rename = "bridge_attached_endpoints")]
    attached_endpoints: u64,
    #[serde(rename = "bridge_free_endpoints")]
    free_endpoints: u64,
}

impl BridgeEndpointCapacity {
    /// Build counts from live usable and attached endpoints.
    #[must_use]
    pub fn new(usable_endpoints: u64, attached_endpoints: u64) -> Self {
        Self {
            usable_endpoints,
            attached_endpoints,
            free_endpoints: usable_endpoints.saturating_sub(attached_endpoints),
        }
    }

    /// IPAM-derived bridge endpoints available to Containers.
    #[must_use]
    pub fn usable_endpoints(&self) -> u64 {
        self.usable_endpoints
    }

    /// Endpoints currently attached to the bridge.
    #[must_use]
    pub fn attached_endpoints(&self) -> u64 {
        self.attached_endpoints
    }

    /// Usable bridge endpoints not currently attached.
    #[must_use]
    pub fn free_endpoints(&self) -> u64 {
        self.free_endpoints
    }
}

impl<'de> Deserialize<'de> for BridgeEndpointCapacity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            bridge_usable_endpoints: u64,
            bridge_attached_endpoints: u64,
            bridge_free_endpoints: u64,
        }

        let wire = Wire::deserialize(deserializer)?;
        let capacity = Self::new(wire.bridge_usable_endpoints, wire.bridge_attached_endpoints);
        if capacity.free_endpoints != wire.bridge_free_endpoints {
            return Err(de::Error::custom("bridge endpoint counts are inconsistent"));
        }
        Ok(capacity)
    }
}

/// Fresh operator telemetry from one Machine. It is not placement scoring data.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MachineTelemetry {
    /// Unix timestamp when this observation began.
    pub observed_at_unix_seconds: u64,
    /// Ployz-managed Docker Container count.
    pub managed_containers: u64,
    /// Host logical CPU count.
    pub cpu_count: u64,
    /// One-minute host load average multiplied by 1,000.
    pub load_average_milli: u64,
    /// Host memory total in bytes.
    pub memory_total_bytes: u64,
    /// Host memory available in bytes.
    pub memory_available_bytes: u64,
    /// Docker-root filesystem size in bytes.
    pub docker_root_total_bytes: u64,
    /// Docker-root filesystem free bytes.
    pub docker_root_free_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inconsistent_endpoint_counts_cannot_deserialize() {
        assert!(
            serde_json::from_str::<BridgeEndpointCapacity>(
                r#"{"bridge_usable_endpoints":5,"bridge_attached_endpoints":2,"bridge_free_endpoints":4}"#,
            )
            .is_err()
        );
    }
}
