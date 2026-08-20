//! Fresh operator telemetry from one Machine.

use serde::{Deserialize, Serialize};

/// Fresh operator telemetry from one Machine. It is not placement scoring data.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MachineTelemetry {
    /// Unix timestamp when this observation began.
    pub observed_at_unix_seconds: u64,
    /// IPAM-derived bridge endpoints available to Containers.
    pub bridge_usable_endpoints: u64,
    /// Endpoints currently attached to the bridge.
    pub bridge_attached_endpoints: u64,
    /// Usable bridge endpoints not currently attached.
    pub bridge_free_endpoints: u64,
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
