use futures_util::future::join_all;
use ployz_core::{
    InspectRequest, MachineFailure, MachineObservation, MachineSuccess, MachineTarget,
    MachineTelemetry, PartialResult, RpcError, op,
};

use super::Client;
use crate::connect::TARGET_RPC_TIMEOUT;

pub(super) async fn observe(
    client: &Client,
    machines: &[MachineObservation],
) -> PartialResult<MachineTelemetry, RpcError> {
    let mut result = PartialResult {
        successes: Vec::new(),
        failures: Vec::new(),
        omissions: Vec::new(),
    };
    let requests = machines.iter().filter_map(|machine| {
        let machine_id = machine.machine.id;
        machine.membership.invites_rpc().then(|| {
            let client = client.clone();
            async move {
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
            }
        })
    });
    result.omissions.extend(
        machines
            .iter()
            .filter(|machine| !machine.membership.invites_rpc())
            .map(|machine| machine.machine.id),
    );
    for (machine_id, response) in join_all(requests).await {
        match response {
            Ok(details) => match details.telemetry {
                Some(telemetry) => result.successes.push(MachineSuccess {
                    machine_id,
                    value: telemetry,
                }),
                None => result.omissions.push(machine_id),
            },
            Err(error) => result.failures.push(MachineFailure { machine_id, error }),
        }
    }
    result
}
