use std::{collections::BTreeMap, future::Future, str::FromStr};

use ployz_core::{
    DockerVolume, DockerVolumeId, DockerVolumeName, MachineFailure, MachineId, MachineName,
    MachineObservation, MachineSuccess, PartialResult, RpcError, RpcErrorCode,
};
use serde::Serialize;
use thiserror::Error;

/// A positive Provisioned Volume bound accepted by Docker's Ployz driver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvisionedVolumeSize(String);

/// Why a Provisioned Volume bound is invalid.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{0}")]
pub struct ProvisionedVolumeSizeError(String);

impl ProvisionedVolumeSize {
    /// Returns the validated spelling expected by Docker's Ployz driver.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl FromStr for ProvisionedVolumeSize {
    type Err = ProvisionedVolumeSizeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || {
            ProvisionedVolumeSizeError(format!(
                "invalid Volume size {value:?}; use a positive integer followed by k, m, g, or t"
            ))
        };
        let (amount, multiplier) = match value.as_bytes().last() {
            Some(b'k') => (&value[..value.len() - 1], 1024_u64),
            Some(b'm') => (&value[..value.len() - 1], 1024_u64.pow(2)),
            Some(b'g') => (&value[..value.len() - 1], 1024_u64.pow(3)),
            Some(b't') => (&value[..value.len() - 1], 1024_u64.pow(4)),
            _ => return Err(invalid()),
        };
        let amount = amount.parse::<u64>().map_err(|_| invalid())?;
        if amount == 0 {
            return Err(ProvisionedVolumeSizeError(
                "Volume size must be greater than zero".into(),
            ));
        }
        amount.checked_mul(multiplier).ok_or_else(|| {
            ProvisionedVolumeSizeError(format!("Volume size {value:?} overflows bytes"))
        })?;
        Ok(Self(value.to_owned()))
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AssignmentError {
    #[error("expected KEY=VALUE, got {0:?}")]
    MissingDelimiter(String),
    #[error("assignment key cannot be empty")]
    EmptyKey,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MachineVolume {
    pub machine_name: MachineName,
    pub volume: DockerVolume,
}

#[must_use]
pub fn filter_volumes(volumes: &[MachineVolume], names: &[DockerVolumeName]) -> Vec<MachineVolume> {
    volumes
        .iter()
        .filter(|volume| names.is_empty() || names.contains(&volume.volume.id.name))
        .cloned()
        .collect()
}

#[must_use]
pub fn machine_volumes(
    machines: &[MachineObservation],
    result: &PartialResult<Vec<DockerVolume>, RpcError>,
) -> Vec<MachineVolume> {
    let names = machines
        .iter()
        .map(|machine| (machine.machine.id, machine.machine.name.clone()))
        .collect::<BTreeMap<MachineId, MachineName>>();
    result
        .successes
        .iter()
        .flat_map(|success| {
            let machine_name = names
                .get(&success.machine_id)
                .cloned()
                .expect("Volume result target came from the Machine snapshot");
            success
                .value
                .iter()
                .cloned()
                .map(move |volume| MachineVolume {
                    machine_name: machine_name.clone(),
                    volume,
                })
        })
        .collect()
}

pub fn parse_assignments<'a>(
    values: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeMap<String, String>, AssignmentError> {
    values
        .into_iter()
        .map(|value| {
            let (key, value) = value
                .split_once('=')
                .ok_or_else(|| AssignmentError::MissingDelimiter(value.to_owned()))?;
            if key.is_empty() {
                return Err(AssignmentError::EmptyKey);
            }
            Ok((key.into(), value.into()))
        })
        .collect()
}

pub async fn remove_volumes_with<F, Fut>(
    volumes: &[MachineVolume],
    force: bool,
    remove: F,
) -> PartialResult<DockerVolumeName, RpcError>
where
    F: Fn(DockerVolumeId, bool) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<(), RpcError>> + Send + 'static,
{
    let mut removals = tokio::task::JoinSet::new();
    for (index, volume) in volumes.iter().enumerate() {
        let id = volume.volume.id.clone();
        let remove = remove.clone();
        removals.spawn(async move {
            let outcome = remove(id.clone(), force).await;
            (index, id, outcome)
        });
    }
    let mut outcomes = Vec::with_capacity(volumes.len());
    while let Some(outcome) = removals.join_next().await {
        outcomes.push(outcome.expect("Volume removal task does not panic"));
    }
    outcomes.sort_by_key(|(index, _, _)| *index);
    let mut result = PartialResult {
        successes: Vec::new(),
        failures: Vec::new(),
        omissions: Vec::new(),
    };
    for (_, id, outcome) in outcomes {
        match outcome {
            Ok(())
            | Err(RpcError {
                code: RpcErrorCode::NotFound,
                ..
            }) => result.successes.push(MachineSuccess {
                machine_id: id.machine_id,
                value: id.name,
            }),
            Err(error) => result.failures.push(MachineFailure {
                machine_id: id.machine_id,
                error,
            }),
        }
    }
    result
}
