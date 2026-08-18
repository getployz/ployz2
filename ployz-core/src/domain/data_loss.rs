//! Data Loss: one named thing an operation will destroy.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::PartialResult;
use crate::{DockerVolume, DockerVolumeId, MachineId, RpcError, RpcErrorCode};

/// One named thing an operation will destroy.
///
/// Identity is per kind: a Docker Volume carries its Machine together with its
/// name. A kind cannot be paired with an identity that does not belong to it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DataLoss {
    /// A Docker Volume identified by its Machine together with its name.
    DockerVolume(DockerVolumeId),
}

/// Live Observation of Data Loss. Not a complete Cluster view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedDataLoss {
    pub data_loss: Vec<DataLoss>,
}

impl ObservedDataLoss {
    /// Data Loss this observer listed for `machine_id`.
    ///
    /// `listed` is the Volume listing for that one Machine.
    ///
    /// # Errors
    ///
    /// Returns the listing error when that Machine failed, or
    /// [`RpcErrorCode::Unavailable`] when this observer produced no listing.
    pub fn from_volume_listing(
        machine_id: &MachineId,
        listed: &PartialResult<Vec<DockerVolume>, RpcError>,
    ) -> Result<Self, RpcError> {
        if let Some(success) = listed.successes.first() {
            return Ok(Self {
                data_loss: success
                    .value
                    .iter()
                    .map(|volume| DataLoss::DockerVolume(volume.id.clone()))
                    .collect(),
            });
        }
        if let Some(failure) = listed.failures.first() {
            return Err(failure.error.clone());
        }
        Err(RpcError {
            code: RpcErrorCode::Unavailable,
            message: format!(
                "Machine {machine_id} did not produce a Volume listing from this observer"
            ),
            details: Value::Null,
        })
    }
}
