use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{DockerVolume, DockerVolumeId, DockerVolumeName, RpcError};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreateVolumeRequest {
    pub name: DockerVolumeName,
    pub driver: String,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListVolumesRequest {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InspectVolumeRequest {
    pub name: DockerVolumeName,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoveVolumeRequest {
    pub name: DockerVolumeName,
    #[serde(default)]
    pub force: bool,
}

/// One Docker Volume whose name is present but whose current detail is unavailable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VolumeObservationFailure {
    pub id: DockerVolumeId,
    pub error: RpcError,
}

/// Partial live inventory for one Machine.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VolumeInventory {
    #[serde(default)]
    pub volumes: Vec<DockerVolume>,
    #[serde(default)]
    pub failures: Vec<VolumeObservationFailure>,
}

/// Successful Docker mutation followed by its independent verification outcome.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verification", rename_all = "snake_case")]
pub enum CreateVolumeReport {
    Verified { volume: DockerVolume },
    Unverified { id: DockerVolumeId, error: RpcError },
}

impl CreateVolumeReport {
    /// Return the verified observation or the created identity and verification failure.
    #[expect(
        clippy::result_large_err,
        reason = "both outcomes are public wire values; boxing would leak allocation into the contract"
    )]
    pub fn into_observation(self) -> Result<DockerVolume, VolumeObservationFailure> {
        match self {
            Self::Verified { volume } => Ok(volume),
            Self::Unverified { id, error } => Err(VolumeObservationFailure { id, error }),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct VolumeRemoved {}
