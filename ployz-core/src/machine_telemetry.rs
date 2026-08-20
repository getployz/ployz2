//! Fresh operator telemetry from one Machine.

use serde::{Deserialize, Deserializer, Serialize, de};

/// Consistent usable, attached, and free endpoint counts for the Ployz bridge.
///
/// Attached endpoints may exceed the currently usable IPAM range when live
/// Docker state is overcommitted. In that state free capacity is zero and
/// [`Self::overcommitted_endpoints`] reports the excess.
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

    /// Attached endpoints beyond the currently usable IPAM range.
    #[must_use]
    pub fn overcommitted_endpoints(&self) -> u64 {
        self.attached_endpoints
            .saturating_sub(self.usable_endpoints)
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
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
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
    memory_total_bytes: u64,
    /// Host memory available in bytes.
    memory_available_bytes: u64,
    /// Docker-root filesystem size in bytes.
    docker_root_total_bytes: u64,
    /// Docker-root filesystem free bytes.
    docker_root_free_bytes: u64,
}

/// A total byte count and a usable subset that cannot exceed it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ByteCapacity {
    total: u64,
    available: u64,
}

impl ByteCapacity {
    /// Build a byte capacity when `available` does not exceed `total`.
    #[must_use]
    pub fn new(total: u64, available: u64) -> Option<Self> {
        (available <= total).then_some(Self { total, available })
    }

    /// Total bytes in the resource.
    #[must_use]
    pub fn total(self) -> u64 {
        self.total
    }

    /// Available bytes in the resource.
    #[must_use]
    pub fn available(self) -> u64 {
        self.available
    }
}

impl MachineTelemetry {
    /// Build telemetry from validated memory and Docker-root capacities.
    #[must_use]
    pub fn new(
        observed_at_unix_seconds: u64,
        managed_containers: u64,
        cpu_count: u64,
        load_average_milli: u64,
        memory: ByteCapacity,
        docker_root: ByteCapacity,
    ) -> Self {
        Self {
            observed_at_unix_seconds,
            managed_containers,
            cpu_count,
            load_average_milli,
            memory_total_bytes: memory.total(),
            memory_available_bytes: memory.available(),
            docker_root_total_bytes: docker_root.total(),
            docker_root_free_bytes: docker_root.available(),
        }
    }

    /// Host memory total in bytes.
    #[must_use]
    pub fn memory_total_bytes(&self) -> u64 {
        self.memory_total_bytes
    }

    /// Host memory available in bytes.
    #[must_use]
    pub fn memory_available_bytes(&self) -> u64 {
        self.memory_available_bytes
    }

    /// Docker-root filesystem size in bytes.
    #[must_use]
    pub fn docker_root_total_bytes(&self) -> u64 {
        self.docker_root_total_bytes
    }

    /// Docker-root filesystem free bytes.
    #[must_use]
    pub fn docker_root_free_bytes(&self) -> u64 {
        self.docker_root_free_bytes
    }
}

impl<'de> Deserialize<'de> for MachineTelemetry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            observed_at_unix_seconds: u64,
            managed_containers: u64,
            cpu_count: u64,
            load_average_milli: u64,
            memory_total_bytes: u64,
            memory_available_bytes: u64,
            docker_root_total_bytes: u64,
            docker_root_free_bytes: u64,
        }

        let wire = Wire::deserialize(deserializer)?;
        let memory = ByteCapacity::new(wire.memory_total_bytes, wire.memory_available_bytes)
            .ok_or_else(|| de::Error::custom("Machine memory counts are inconsistent"))?;
        let docker_root =
            ByteCapacity::new(wire.docker_root_total_bytes, wire.docker_root_free_bytes)
                .ok_or_else(|| de::Error::custom("Docker-root byte counts are inconsistent"))?;
        Ok(Self::new(
            wire.observed_at_unix_seconds,
            wire.managed_containers,
            wire.cpu_count,
            wire.load_average_milli,
            memory,
            docker_root,
        ))
    }
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

    #[test]
    fn overcommitted_endpoint_counts_are_explicit() {
        let capacity = BridgeEndpointCapacity::new(1, 3);
        assert_eq!(capacity.free_endpoints(), 0);
        assert_eq!(capacity.overcommitted_endpoints(), 2);
    }

    #[test]
    fn inconsistent_machine_byte_counts_cannot_deserialize() {
        assert!(
            serde_json::from_str::<MachineTelemetry>(
                r#"{"observed_at_unix_seconds":0,"managed_containers":0,"cpu_count":1,"load_average_milli":0,"memory_total_bytes":1,"memory_available_bytes":2,"docker_root_total_bytes":1,"docker_root_free_bytes":0}"#,
            )
            .is_err()
        );
    }
}
