//! Keep hosted public DNS aligned with live Caddy Machine membership.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use futures_util::future::join_all;
use ployz_core::{
    CADDY_VERIFY_PATH, DnsRecord, DnsRecordType, Machine, MachineId, MachineObservation,
    ManagementAddress, MembershipObservation, QualifiedService, synthesize_membership,
};
use reqwest::{Client, StatusCode, redirect::Policy};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::{
    corrosion::{AdminClient, MembershipState, ReplicatedStore},
    hosted_dns::{HostedDns, Reservation},
};

const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const REACHABILITY_TIMEOUT: Duration = Duration::from_secs(5);

/// Keep the public wildcard aligned with this Machine's live membership view.
pub(crate) async fn run(
    store: ReplicatedStore,
    admin: AdminClient,
    local_id: MachineId,
    shutdown: CancellationToken,
) -> io::Result<()> {
    let hosted = HostedDns::new();
    let http = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .timeout(REACHABILITY_TIMEOUT)
        .build()
        .map_err(io::Error::other)?;
    let mut interval = tokio::time::interval(REFRESH_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut published = None;
    loop {
        tokio::select! {
            _ = interval.tick() => match projection(&store, &admin, &local_id, &http).await {
                Ok(None) => published = None,
                Ok(Some(next)) if published.as_ref() != Some(&next) => {
                    match hosted.replace_records(&next.0, &next.1).await {
                        Ok(_) => published = Some(next),
                        Err(error) => eprintln!("failed to update hosted DNS membership projection: {error}"),
                    }
                }
                Ok(Some(_)) => {}
                Err(error) => eprintln!("failed to rebuild hosted DNS membership projection: {error}"),
            },
            () = shutdown.cancelled() => return Ok(()),
        }
    }
}

async fn projection(
    store: &ReplicatedStore,
    admin: &AdminClient,
    local_id: &MachineId,
    http: &Client,
) -> Result<Option<(Reservation, Vec<DnsRecord>)>, ProjectionError> {
    let Some(reservation) = store.domain_reservation().await? else {
        return Ok(None);
    };
    let (machines, containers, states) = tokio::try_join!(
        store.machines(),
        store.containers(),
        admin.membership_states()
    )?;
    let observations = synthesize_membership(
        machines.observations,
        local_id,
        &membership_by_address(states),
    );
    if public_dns_writer(&observations) != Some(*local_id) {
        return Ok(None);
    }
    let caddy_machines = containers
        .observations
        .into_iter()
        .filter(|container| container.identity() == QualifiedService::system_caddy())
        .map(|container| container.machine_id)
        .collect::<BTreeSet<_>>();
    if caddy_machines.is_empty() {
        return Err(ProjectionError::NoReachableMachines);
    }
    let candidates = public_machines(observations, &caddy_machines);
    let records = if candidates.is_empty() {
        Vec::new()
    } else {
        let reachable = probe_machines(http, candidates).await;
        if reachable.is_empty() {
            return Err(ProjectionError::NoReachableMachines);
        }
        records_from_machines(reachable)
    };
    Ok(Some((reservation, records)))
}

fn public_dns_writer(observations: &[MachineObservation]) -> Option<MachineId> {
    // All Machines with the same membership view select the same writer, and
    // the next eligible Machine takes over when that writer leaves membership.
    observations
        .iter()
        .filter(|observation| observation.membership.invites_rpc())
        .map(|observation| observation.machine.id)
        .min()
}

fn membership_by_address(
    states: impl IntoIterator<Item = MembershipState>,
) -> BTreeMap<ManagementAddress, MembershipObservation> {
    states
        .into_iter()
        .filter_map(|state| match state.address.ip() {
            IpAddr::V6(address) => Some((ManagementAddress(address), state.membership)),
            IpAddr::V4(_) => None,
        })
        .collect()
}

fn public_machines(
    observations: impl IntoIterator<Item = MachineObservation>,
    caddy_machines: &BTreeSet<MachineId>,
) -> Vec<Machine> {
    observations
        .into_iter()
        .filter_map(|observation| {
            (observation.membership.invites_rpc()
                && caddy_machines.contains(&observation.machine.id)
                && observation.machine.public_ip.is_some())
            .then_some(observation.machine)
        })
        .collect()
}

async fn probe_machines(client: &Client, machines: Vec<Machine>) -> Vec<Machine> {
    join_all(machines.into_iter().map(|machine| async move {
        let address = SocketAddr::new(
            machine
                .public_ip
                .expect("public Machines have a public address"),
            80,
        );
        let matches = match client
            .get(format!("http://{address}{CADDY_VERIFY_PATH}"))
            .send()
            .await
        {
            Ok(response) if response.status() == StatusCode::OK => response
                .bytes()
                .await
                .is_ok_and(|body| body.as_ref() == machine.id.as_str().as_bytes()),
            Ok(_) | Err(_) => false,
        };
        matches.then_some(machine)
    }))
    .await
    .into_iter()
    .flatten()
    .collect()
}

fn records_from_machines(machines: impl IntoIterator<Item = Machine>) -> Vec<DnsRecord> {
    let mut ipv4 = BTreeSet::new();
    let mut ipv6 = BTreeSet::new();
    for machine in machines {
        match machine.public_ip {
            Some(IpAddr::V4(address)) => {
                ipv4.insert(address.to_string());
            }
            Some(IpAddr::V6(address)) => {
                ipv6.insert(address.to_string());
            }
            None => {}
        }
    }
    let mut records = Vec::new();
    if !ipv4.is_empty() {
        records.push(DnsRecord {
            name: "*".into(),
            record_type: DnsRecordType::A,
            values: ipv4.into_iter().collect(),
        });
    }
    if !ipv6.is_empty() {
        records.push(DnsRecord {
            name: "*".into(),
            record_type: DnsRecordType::Aaaa,
            values: ipv6.into_iter().collect(),
        });
    }
    records
}

#[derive(Debug, Error)]
enum ProjectionError {
    #[error(transparent)]
    Store(#[from] crate::corrosion::Error),
    #[error("no publicly reachable Caddy Machines found")]
    NoReachableMachines,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use ployz_core::{Machine, MachineId, MachineObservation, MembershipObservation};

    use super::{public_dns_writer, public_machines, records_from_machines};

    #[test]
    fn public_records_follow_all_membership_states_in_both_directions() {
        let mut observation = MachineObservation {
            machine: serde_json::from_value::<Machine>(serde_json::json!({
                "id": "4".repeat(32),
                "name": "machine-4",
                "subnet": "10.210.4.0/24",
                "management_address": "fdcc::4",
                "public_key": vec![4; 32],
                "public_ip": "192.0.2.4"
            }))
            .unwrap(),
            membership: MembershipObservation::Unknown,
            selected_endpoint: None,
            rtt: None,
        };
        let caddy_machines = BTreeSet::from([observation.machine.id]);

        for (membership, published) in [
            (MembershipObservation::Unknown, false),
            (MembershipObservation::Suspect, true),
            (MembershipObservation::Down, false),
            (MembershipObservation::Up, true),
            (MembershipObservation::Unknown, false),
        ] {
            observation.membership = membership;
            assert_eq!(
                !records_from_machines(public_machines([observation.clone()], &caddy_machines,))
                    .is_empty(),
                published
            );
        }
    }

    #[test]
    fn lowest_up_or_suspect_machine_is_the_only_writer() {
        let observation = |seed: char, membership| MachineObservation {
            machine: serde_json::from_value::<Machine>(serde_json::json!({
                "id": seed.to_string().repeat(32),
                "name": format!("machine-{seed}"),
                "subnet": format!("10.210.{}.0/24", seed.to_digit(10).unwrap()),
                "management_address": format!("fdcc::{seed}"),
                "public_key": vec![seed as u8; 32]
            }))
            .unwrap(),
            membership,
            selected_endpoint: None,
            rtt: None,
        };
        let mut observations = vec![
            observation('1', MembershipObservation::Down),
            observation('2', MembershipObservation::Suspect),
            observation('3', MembershipObservation::Up),
        ];
        assert_eq!(
            public_dns_writer(&observations),
            Some(MachineId::parse("2".repeat(32)).unwrap())
        );

        observations.first_mut().unwrap().membership = MembershipObservation::Up;
        assert_eq!(
            public_dns_writer(&observations),
            Some(MachineId::parse("1".repeat(32)).unwrap())
        );
    }
}
