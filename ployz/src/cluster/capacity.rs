//! Concurrent targeted bridge-capacity observations.

use std::collections::BTreeMap;

use futures_util::future::join_all;
use ployz_core::{
    BridgeEndpointCapacity, InspectRequest, MachineId, MachineObservation, MachineTarget, op,
};

use super::Client;
use crate::connect::TARGET_RPC_TIMEOUT;

pub(super) async fn observe(
    client: &Client,
    machines: &[MachineObservation],
) -> BTreeMap<MachineId, BridgeEndpointCapacity> {
    let requests = machines.iter().filter_map(|machine| {
        let machine_id = machine.machine.id;
        machine.membership.invites_rpc().then(|| {
            let client = client.clone();
            async move {
                let result = client
                    .invoke::<op::Inspect>(
                        InspectRequest {
                            include_bridge_capacity: true,
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
    join_all(requests)
        .await
        .into_iter()
        .filter_map(|(machine_id, response)| {
            response
                .ok()?
                .bridge_capacity
                .map(|capacity| (machine_id, capacity))
        })
        .collect()
}
