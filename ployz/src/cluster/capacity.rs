use std::collections::HashMap;

use ployz_core::{
    InspectRequest, MachineFailure, MachineObservation, MachineSuccess, MachineTarget,
    MachineTelemetry, PartialResult, RpcError, RpcErrorCode, op,
};
use serde_json::Value;
use tokio::task::JoinSet;

use super::Client;
use crate::connect::TARGET_RPC_TIMEOUT;

pub(super) async fn observe(
    client: &Client,
    machines: &[MachineObservation],
) -> PartialResult<MachineTelemetry, RpcError> {
    let mut tasks = JoinSet::new();
    let mut task_targets = HashMap::new();
    let mut result = PartialResult {
        successes: Vec::new(),
        failures: Vec::new(),
        omissions: Vec::new(),
    };
    for machine in machines {
        let machine_id = machine.machine.id;
        if !machine.membership.invites_rpc() {
            result.omissions.push(machine_id);
            continue;
        }
        let client = client.clone();
        let task = tasks.spawn(async move {
            let result = client
                .invoke::<op::Inspect>(
                    InspectRequest {
                        include_telemetry: true,
                        ..Default::default()
                    },
                    &MachineTarget::from(&machine_id),
                    Some(TARGET_RPC_TIMEOUT),
                )
                .await;
            (machine_id, result)
        });
        task_targets.insert(task.id(), machine_id);
    }
    while let Some(joined) = tasks.join_next_with_id().await {
        match joined {
            Ok((task_id, (machine_id, Ok(details)))) => {
                task_targets.remove(&task_id);
                match details.telemetry {
                    Some(telemetry) => result.successes.push(MachineSuccess {
                        machine_id,
                        value: telemetry,
                    }),
                    None => result.omissions.push(machine_id),
                }
            }
            Ok((task_id, (machine_id, Err(error)))) => {
                task_targets.remove(&task_id);
                result.failures.push(MachineFailure { machine_id, error });
            }
            Err(error) => {
                if let Some(machine_id) = task_targets.remove(&error.id()) {
                    result.failures.push(MachineFailure {
                        machine_id,
                        error: RpcError {
                            code: RpcErrorCode::Unavailable,
                            message: error.to_string(),
                            details: Value::Null,
                        },
                    });
                }
            }
        }
    }
    result
}
