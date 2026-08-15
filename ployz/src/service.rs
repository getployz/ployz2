use std::time::Duration;

use ployz_core::{
    ContainerAction, ContainerId, CreateVolumeRequest, ListContainersRequest, MachineFailure,
    MachineId, MachineSelector, MachineSuccess, PartialResult, RemoveContainerRequest, RpcError,
    RpcErrorCode, ServiceSelectorError, StartContainerRequest, StopContainerRequest, op,
};
use thiserror::Error;

use crate::connect::{Client, ConnectError, TARGET_RPC_TIMEOUT};

#[derive(Debug, Error)]
pub enum ServiceClientError {
    #[error(transparent)]
    Connect(#[from] ConnectError),
    #[error(transparent)]
    Selector(#[from] ServiceSelectorError),
}

pub struct LifecycleResult {
    pub observations: PartialResult<Vec<ployz_core::ContainerObservation>, RpcError>,
    pub outcomes: PartialResult<ContainerId, ContainerOperationFailure>,
}

#[derive(Clone, Debug)]
pub struct ContainerOperationFailure {
    pub container_id: ContainerId,
    pub error: RpcError,
}

pub(crate) async fn create_volume_on_machine(
    client: &Client,
    machine_id: &MachineId,
    request: CreateVolumeRequest,
) -> Result<(), RpcError> {
    client
        .invoke::<op::CreateVolume>(
            request,
            &MachineSelector::from(machine_id),
            Some(TARGET_RPC_TIMEOUT),
        )
        .await
        .map(|_| ())
}

pub(crate) async fn list_on_machine(
    client: Client,
    machine_id: MachineId,
) -> Result<MachineSuccess<Vec<ployz_core::ContainerObservation>>, MachineFailure<RpcError>> {
    client
        .invoke::<op::ListContainers>(
            ListContainersRequest {},
            &MachineSelector::from(&machine_id),
            Some(TARGET_RPC_TIMEOUT),
        )
        .await
        .map(|list| MachineSuccess {
            machine_id: machine_id.clone(),
            value: list.containers,
        })
        .map_err(|error| MachineFailure { machine_id, error })
}

pub(crate) async fn change_on_machine(
    client: Client,
    machine_id: MachineId,
    container_id: ContainerId,
    action: ContainerAction,
    signal: Option<String>,
    grace_period_seconds: Option<i32>,
) -> Result<MachineSuccess<ContainerId>, MachineFailure<ContainerOperationFailure>> {
    match change_container_rpc(
        &client,
        &machine_id,
        &container_id,
        action,
        signal,
        grace_period_seconds,
    )
    .await
    {
        Ok(()) => Ok(MachineSuccess {
            machine_id,
            value: container_id,
        }),
        Err(error) => Err(MachineFailure {
            machine_id,
            error: ContainerOperationFailure {
                container_id,
                error,
            },
        }),
    }
}

pub(crate) async fn change_container_rpc(
    client: &Client,
    machine_id: &MachineId,
    container_id: &ContainerId,
    action: ContainerAction,
    signal: Option<String>,
    grace_period_seconds: Option<i32>,
) -> Result<(), RpcError> {
    let target = MachineSelector::from(machine_id);
    if matches!(action, ContainerAction::Stop | ContainerAction::Remove) {
        accept_stop_result(
            action,
            client
                .invoke::<op::StopContainer>(
                    StopContainerRequest {
                        container_id: container_id.clone(),
                        signal,
                        grace_period_seconds,
                    },
                    &target,
                    stop_rpc_timeout(grace_period_seconds),
                )
                .await
                .map(|_| ()),
        )?;
    }
    match action {
        ContainerAction::Start => client
            .invoke::<op::StartContainer>(
                StartContainerRequest {
                    container_id: container_id.clone(),
                },
                &target,
                Some(TARGET_RPC_TIMEOUT),
            )
            .await
            .map(|_| ()),
        ContainerAction::Stop => Ok(()),
        ContainerAction::Remove => remove_container_rpc(client, machine_id, container_id).await,
    }
}

pub(crate) async fn remove_container_rpc(
    client: &Client,
    machine_id: &MachineId,
    container_id: &ContainerId,
) -> Result<(), RpcError> {
    client
        .invoke::<op::RemoveContainer>(
            RemoveContainerRequest {
                container_id: container_id.clone(),
                remove_volumes: true,
                force: false,
            },
            &MachineSelector::from(machine_id),
            Some(TARGET_RPC_TIMEOUT),
        )
        .await
        .map(|_| ())
}

fn stop_rpc_timeout(grace_period_seconds: Option<i32>) -> Option<Duration> {
    match grace_period_seconds {
        Some(seconds) if seconds < 0 => None,
        Some(seconds) => Some(TARGET_RPC_TIMEOUT + Duration::from_secs(seconds as u64)),
        None => None,
    }
}

fn accept_stop_result(
    action: ContainerAction,
    result: Result<(), RpcError>,
) -> Result<(), RpcError> {
    match result {
        Err(error) if action == ContainerAction::Remove && error.code == RpcErrorCode::NotFound => {
            Ok(())
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn unspecified_or_negative_stop_timeout_has_no_rpc_deadline() {
        assert_eq!(stop_rpc_timeout(Some(-1)), None);
        assert_eq!(stop_rpc_timeout(None), None);
        assert_eq!(
            stop_rpc_timeout(Some(5)),
            Some(TARGET_RPC_TIMEOUT + Duration::from_secs(5))
        );
    }

    #[test]
    fn remove_tolerates_a_missing_preliminary_stop_target() {
        let missing = RpcError {
            code: RpcErrorCode::NotFound,
            message: "gone".into(),
            details: Value::Null,
        };

        assert!(accept_stop_result(ContainerAction::Remove, Err(missing.clone())).is_ok());
        assert_eq!(
            accept_stop_result(ContainerAction::Stop, Err(missing.clone()))
                .unwrap_err()
                .code,
            RpcErrorCode::NotFound
        );
        assert_eq!(
            accept_stop_result(
                ContainerAction::Remove,
                Err(RpcError {
                    code: RpcErrorCode::Internal,
                    ..missing
                })
            )
            .unwrap_err()
            .code,
            RpcErrorCode::Internal
        );
    }
}
