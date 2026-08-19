//! Local Machine operations as Live Observation and membership changes
//! from this Machine's Local Machine Phase.

use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use ployz_core::{
    InitializeRequest, Initialized, InspectRequest, JoinAccepted, JoinRequest, LocalMachinePhase,
    LocalMachineRemoved, Machine, MachineDetails, MachineId, MachineIdentity, MachineList,
    MachineObservation, MachineRemoved, MachineRpcClient, MachineToken, MachineTokenRequest,
    MachineUpdated, ManagementAddress, MembershipObservation, PublicIpDiscovery, RegisterRequest,
    Registered, RemoveLocalMachineRequest, RemoveMachineRequest, ResetAccepted, RpcError,
    RpcResponseBody, RttObservation, RttStatistics, SelectedEndpoint, UpdateMachineRequest,
    WireGuardInspected, associate_wireguard_peers, op, synthesize_membership,
};
use thiserror::Error;
use tokio::sync::watch;
use tonic::transport::Endpoint;

use super::{LocalMachineRecord, LocalMachineStore, StoreError, local_runtime};
use crate::{
    corrosion::{AdminClient, MembershipState, ReplicatedStore},
    docker::ContainerRuntime,
    network::{
        MACHINE_API_PORT, NetworkError, allocate_machine_subnet, discover_network,
        inspect_wireguard_device, management_address,
    },
};

/// Live Observation and membership operations for this Machine.
#[derive(Clone)]
pub struct LocalMachine {
    store: Arc<Mutex<LocalMachineStore>>,
    restart: watch::Sender<bool>,
    cluster: Option<ClusterContext>,
    containers: Option<ContainerRuntime>,
}

#[derive(Clone)]
struct ClusterContext {
    replicated: ReplicatedStore,
    admin: AdminClient,
    machine_api_port: u16,
}

/// Metadata on a forwarded Machine-to-Machine Register. The named Allocator
/// admits locally and does not forward again.
pub(crate) const REGISTER_FORWARDED_METADATA: &str = "x-ployz-register-forwarded";

enum RegisterHop {
    Origin,
    Forwarded,
}

/// Entry-local admin membership, RTT samples, and selected endpoints.
///
/// Projected onto the current replicated Machine snapshot at assemble time.
/// Missing telemetry is not a delete of the replicated Machine.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RuntimeWatchTelemetry {
    pub states: BTreeMap<ManagementAddress, MembershipObservation>,
    pub selected_endpoints: BTreeMap<MachineId, SelectedEndpoint>,
    pub rtts: Vec<RttObservation>,
}

impl RuntimeWatchTelemetry {
    /// Membership, selected endpoints, and RTT for the current replicated Machines.
    pub(crate) fn overlay(
        &self,
        machines: Vec<Machine>,
        entry_id: &MachineId,
    ) -> Vec<MachineObservation> {
        let rtt = rtts_by_machine(&machines, &self.rtts);
        let mut observations = synthesize_membership(machines, entry_id, &self.states);
        for observation in &mut observations {
            observation.selected_endpoint = self
                .selected_endpoints
                .get(&observation.machine.id)
                .copied();
            observation.rtt = rtt.get(&observation.machine.id).cloned();
        }
        observations
    }
}

/// Failures from Local Machine operations. The RPC adapter maps these once.
#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("Machine is not participating")]
    NotParticipating,
    #[error("Cluster store is not available")]
    ClusterStoreUnavailable,
    #[error("Cluster is not available")]
    ClusterUnavailable,
    #[error("Docker is not available")]
    DockerUnavailable,
    #[error("Machine name or public key already exists")]
    DuplicateMachine,
    #[error("at least one Machine update is required")]
    EmptyUpdate,
    #[error("local Machine record lock poisoned")]
    LockPoisoned,
    #[error(transparent)]
    Cluster(#[from] crate::corrosion::Error),
    #[error(transparent)]
    Network(#[from] NetworkError),
    #[error(transparent)]
    Docker(#[from] crate::docker::Error),
    #[error("Machine is not the Allocator")]
    NotAllocator,
    #[error("Allocator is unreachable")]
    AllocatorUnreachable,
    #[error(transparent)]
    Remote(RpcError),
    #[error(transparent)]
    Codec(#[from] ployz_core::CodecError),
    #[error("{0}")]
    Cleanup(String),
}

impl LocalMachine {
    /// An uninitialized Local Machine with no Cluster or Docker collaborators.
    #[must_use]
    pub fn new(store: Arc<Mutex<LocalMachineStore>>, restart: watch::Sender<bool>) -> Self {
        Self {
            store,
            restart,
            cluster: None,
            containers: None,
        }
    }

    #[must_use]
    pub(crate) fn with_cluster(mut self, cluster: Option<(ReplicatedStore, AdminClient)>) -> Self {
        self.cluster = cluster.map(|(replicated, admin)| ClusterContext {
            replicated,
            admin,
            machine_api_port: MACHINE_API_PORT,
        });
        self
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_machine_api_port(mut self, port: u16) -> Self {
        if let Some(cluster) = &mut self.cluster {
            cluster.machine_api_port = port;
        }
        self
    }

    #[must_use]
    pub(crate) fn with_containers(mut self, containers: Option<ContainerRuntime>) -> Self {
        self.containers = containers;
        self
    }

    pub(crate) fn has_cluster(&self) -> bool {
        self.cluster.is_some()
    }

    pub(crate) fn containers(&self) -> Option<&ContainerRuntime> {
        self.containers.as_ref()
    }

    /// The persisted Local Machine record.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LockPoisoned`] when the local record lock is poisoned.
    pub fn record(&self) -> Result<LocalMachineRecord, Error> {
        Ok(self.lock_store()?.record().clone())
    }

    fn lock_store(&self) -> Result<MutexGuard<'_, LocalMachineStore>, Error> {
        self.store.lock().map_err(|_| Error::LockPoisoned)
    }

    pub(crate) fn replicated(&self) -> Result<&ReplicatedStore, Error> {
        self.cluster
            .as_ref()
            .map(|cluster| &cluster.replicated)
            .ok_or(Error::ClusterStoreUnavailable)
    }

    /// Entry-local membership and RTT from the same admin source as ListMachines
    /// and inspect RTT.
    ///
    /// Returns `None` when the admin socket cannot be read so Watch can keep
    /// replicated rows.
    pub(crate) async fn runtime_watch_telemetry(&self) -> Option<RuntimeWatchTelemetry> {
        let Some(cluster) = &self.cluster else {
            return None;
        };
        let Ok(local) = self.record() else {
            return None;
        };
        let (states, rtts) = read_admin(&cluster.admin).await?;
        Some(RuntimeWatchTelemetry {
            states: membership_states_by_address(states),
            rtts,
            selected_endpoints: local.selected_endpoints,
        })
    }

    /// Live Observation of this Machine's identity, Local Machine Phase, and
    /// advertised reachability.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LockPoisoned`] when the local record lock is poisoned,
    /// [`Error::Network`] when endpoint discovery fails, [`Error::Cluster`]
    /// when store version or RTT lookup fails, and [`Error::ClusterUnavailable`]
    /// when RTTs are requested without a Cluster.
    pub async fn inspect(&self, request: InspectRequest) -> Result<MachineDetails, Error> {
        let record = self.record()?;
        let advertised_endpoints = if !request.advertised_endpoints.is_empty() {
            request.advertised_endpoints
        } else if let Some(machine) = record.machine() {
            machine.advertised_endpoints.clone()
        } else {
            discover_network(
                request.wireguard_port,
                request
                    .public_ip_override
                    .map_or(PublicIpDiscovery::Auto, PublicIpDiscovery::Override),
            )
            .await?
            .endpoints
        };
        let store_version = match &self.cluster {
            Some(cluster) => cluster.replicated.version().await?,
            None => BTreeMap::new(),
        };
        let rtts = if request.include_rtts && record.phase() == LocalMachinePhase::Participating {
            let cluster = self.cluster.as_ref().ok_or(Error::ClusterUnavailable)?;
            machine_rtts(&cluster.admin, &cluster.replicated).await?
        } else {
            Vec::new()
        };
        Ok(MachineDetails {
            id: record.id(),
            phase: record.phase(),
            machine: record.machine().cloned(),
            public_key: record.wireguard_private_key.public_key(),
            advertised_endpoints,
            store_version,
            rtts,
        })
    }

    /// Live Observation of the token a peer uses to register this Machine.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LockPoisoned`] when the local record lock is poisoned
    /// and [`Error::Network`] when endpoint discovery fails.
    pub async fn machine_token(&self, request: MachineTokenRequest) -> Result<MachineToken, Error> {
        let record = self.record()?;
        let private_key = record.wireguard_private_key;
        let discovered = discover_network(request.wireguard_port, request.public_ip).await?;
        Ok(MachineToken {
            public_key: private_key.public_key(),
            public_ip: discovered.public_ip,
            advertised_endpoints: if request.advertised_endpoints.is_empty() {
                discovered.endpoints
            } else {
                request.advertised_endpoints
            },
            runtime: local_runtime(),
        })
    }

    /// Live Observation of the WireGuard device, enriched with Membership
    /// Observation when a Cluster is available.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Network`] when the device cannot be inspected.
    pub async fn inspect_wireguard(&self) -> Result<WireGuardInspected, Error> {
        let device = inspect_wireguard_device()?;
        let Some(cluster) = &self.cluster else {
            return Ok(WireGuardInspected { device });
        };
        let machines = match cluster.replicated.machines().await {
            Ok(snapshot) => snapshot.observations,
            Err(error) => {
                eprintln!("WireGuard Machine enrichment is unavailable: {error}");
                Vec::new()
            }
        };
        let rtts = match machine_rtts(&cluster.admin, &cluster.replicated).await {
            Ok(rtts) => rtts,
            Err(error) => {
                eprintln!("WireGuard RTT enrichment is unavailable: {error}");
                Vec::new()
            }
        }
        .into_iter()
        .filter_map(|observation| {
            observation
                .machine
                .map(|machine| (machine.id, observation.statistics))
        })
        .collect();
        Ok(WireGuardInspected {
            device: associate_wireguard_peers(device, &machines, &rtts),
        })
    }

    /// Persist this Machine as the first participant and request restart.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LockPoisoned`] when the local record lock is poisoned
    /// and [`Error::Store`] when initialize is not legal in the current phase.
    pub fn initialize(&self, request: InitializeRequest) -> Result<Initialized, Error> {
        let machine = self.lock_store()?.initialize(
            request.name,
            request.cluster_network,
            request.public_ip,
            request.advertised_endpoints,
            request.wireguard_mtu,
        )?;
        tracing::info!(
            name = machine.name.as_str(),
            id = machine.id.as_str(),
            "initialize accepted"
        );
        self.restart.send_replace(true);
        Ok(Initialized { machine })
    }

    /// Assign a new Machine into the Cluster from this participating Machine.
    ///
    /// If cluster KV names another Machine as Allocator, Register is forwarded
    /// one hop to that Machine and that `Registered` payload is returned.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when endpoints are missing, [`Error::NotParticipating`]
    /// when this Machine is not participating, [`Error::ClusterStoreUnavailable`]
    /// when the Cluster store is missing, [`Error::NotAllocator`] when this
    /// Machine is not named as Allocator and must not admit locally,
    /// [`Error::AllocatorUnreachable`] when the named Allocator cannot be
    /// reached, [`Error::DuplicateMachine`] when the name or public key already
    /// exists, [`Error::Network`] when subnet allocation fails,
    /// [`Error::Remote`] when the Allocator returns a domain error, and
    /// [`Error::Cluster`] when replicated I/O fails.
    pub async fn register(&self, request: RegisterRequest) -> Result<Registered, Error> {
        self.register_at(request, RegisterHop::Origin).await
    }

    pub(crate) async fn admit_register(
        &self,
        request: RegisterRequest,
    ) -> Result<Registered, Error> {
        self.register_at(request, RegisterHop::Forwarded).await
    }

    async fn register_at(
        &self,
        request: RegisterRequest,
        hop: RegisterHop,
    ) -> Result<Registered, Error> {
        if request.advertised_endpoints.is_empty() {
            return Err(StoreError::MissingEndpoints.into());
        }
        let me = {
            let record = self.record()?;
            if record.phase() != LocalMachinePhase::Participating {
                return Err(Error::NotParticipating);
            }
            record.id()
        };
        let replicated = self.replicated()?;
        match replicated.allocator().await? {
            Some(allocator) if allocator == me => self.admit_local_register(request).await,
            Some(allocator) => match hop {
                RegisterHop::Origin => self.forward_register(allocator, request).await,
                RegisterHop::Forwarded => Err(Error::NotAllocator),
            },
            None => Err(Error::NotAllocator),
        }
    }

    async fn admit_local_register(&self, request: RegisterRequest) -> Result<Registered, Error> {
        let replicated = self.replicated()?;
        let snapshot = replicated.machines().await?;
        if snapshot
            .observations
            .iter()
            .any(|machine| machine.name == request.name || machine.public_key == request.public_key)
        {
            return Err(Error::DuplicateMachine);
        }
        let network = replicated.cluster_network().await?;
        let assigned_machine = Machine {
            id: MachineId::random(),
            name: request.name,
            subnet: allocate_machine_subnet(
                network,
                snapshot.observations.iter().map(|machine| machine.subnet),
            )?,
            management_address: management_address(request.public_key),
            public_key: request.public_key,
            public_ip: request.public_ip,
            advertised_endpoints: request.advertised_endpoints,
            runtime: request.runtime,
        };
        // TODO(UT-140): the imperative registration is deliberately unfenced and has no rollback.
        replicated.publish_local_machine(&assigned_machine).await?;
        let target_versions = replicated.version().await?;
        let visible_peers = replicated
            .machines()
            .await?
            .observations
            .into_iter()
            .filter(|machine| machine.id != assigned_machine.id)
            .collect();
        Ok(Registered {
            assigned_machine,
            visible_peers,
            target_versions,
        })
    }

    async fn forward_register(
        &self,
        allocator: MachineId,
        request: RegisterRequest,
    ) -> Result<Registered, Error> {
        let Some(target) = self.replicated()?.machine(allocator.as_str()).await? else {
            return Err(Error::AllocatorUnreachable);
        };
        let port = self
            .cluster
            .as_ref()
            .expect("forwarding requires the Cluster store used to read the Allocator")
            .machine_api_port;
        let endpoint =
            Endpoint::from_shared(format!("http://[{}]:{}", target.management_address.0, port))
                .expect("Allocator management address forms a URL")
                .connect_timeout(Duration::from_secs(10));
        let mut client = MachineRpcClient::new(
            endpoint
                .connect()
                .await
                .map_err(|_| Error::AllocatorUnreachable)?,
        );
        let mut outbound = tonic::Request::new(op::Register::into_request(request).encode()?);
        outbound.metadata_mut().insert(
            REGISTER_FORWARDED_METADATA,
            "1".parse().expect("ASCII metadata"),
        );
        let payload = client
            .register(outbound)
            .await
            .map_err(|_| Error::AllocatorUnreachable)?
            .into_inner();
        let response = payload.decode_response()?;
        if let RpcResponseBody::Error(error) = &response.body {
            return Err(Error::Remote(error.clone()));
        }
        Ok(response.decode::<op::Register>()?)
    }

    /// Persist a join assignment and request restart.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LockPoisoned`] when the local record lock is poisoned
    /// and [`Error::Store`] when join is not legal in the current phase.
    pub fn join(&self, request: JoinRequest) -> Result<JoinAccepted, Error> {
        let mut store = self.lock_store()?;
        store.join(
            request.registration.assigned_machine,
            request.registration.visible_peers,
            request.registration.target_versions,
            request.wireguard_mtu,
        )?;
        let machine = store
            .record()
            .machine()
            .expect("join persisted the assigned Machine");
        tracing::info!(
            name = machine.name.as_str(),
            id = machine.id.as_str(),
            "join accepted"
        );
        drop(store);
        self.restart.send_replace(true);
        Ok(JoinAccepted {})
    }

    /// Membership Observation of Machines visible from this participating Machine.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotParticipating`] when this Machine is not
    /// participating, [`Error::ClusterStoreUnavailable`] when the Cluster
    /// store is missing, [`Error::LockPoisoned`] when the local record lock is
    /// poisoned, and [`Error::Cluster`] when replicated I/O fails.
    pub async fn list_machines(&self) -> Result<MachineList, Error> {
        let local = self.record()?;
        if local.phase() != LocalMachinePhase::Participating {
            return Err(Error::NotParticipating);
        }
        let replicated = self.replicated()?;
        let machines = replicated.machines().await?.observations;
        let states = match &self.cluster {
            Some(cluster) => cluster.admin.membership_states().await?,
            None => Vec::new(),
        };
        let states = membership_states_by_address(states);
        let entry_id = local.id();
        Ok(MachineList {
            machines: RuntimeWatchTelemetry {
                states,
                selected_endpoints: local.selected_endpoints,
                rtts: Vec::new(),
            }
            .overlay(machines, &entry_id),
        })
    }

    /// Apply a local Machine update and best-effort publish it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptyUpdate`] when no field is set,
    /// [`Error::ClusterStoreUnavailable`] when the Cluster store is missing,
    /// [`Error::LockPoisoned`] when the local record lock is poisoned,
    /// [`Error::Store`] when the update is not legal, and [`Error::Cluster`]
    /// when listing visible Machines fails.
    pub async fn update(&self, request: UpdateMachineRequest) -> Result<MachineUpdated, Error> {
        if request.update.is_empty() {
            return Err(Error::EmptyUpdate);
        }
        let replicated = self.replicated()?;
        let visible = replicated.machines().await?.observations;
        let publication = replicated.machine_publication().await;
        let machine = self.lock_store()?.update(request.update, &visible)?;
        if let Err(error) = publication.publish(&machine).await {
            eprintln!("failed to publish updated local Machine: {error}");
        }
        Ok(MachineUpdated { machine })
    }

    /// Remove a peer Machine from the Cluster. Restarting when the target is
    /// this Machine in the resetting Local Machine Phase.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ClusterStoreUnavailable`] when the Cluster store is
    /// missing, [`Error::Cluster`] when the replicated delete fails, and
    /// [`Error::LockPoisoned`] when the local record lock is poisoned.
    pub async fn remove_peer(
        &self,
        request: RemoveMachineRequest,
    ) -> Result<MachineRemoved, Error> {
        self.replicated()?
            .remove_machine(&request.machine_id)
            .await?;
        let local = self.record()?;
        if local.id() == request.machine_id && local.phase() == LocalMachinePhase::Resetting {
            self.restart.send_replace(true);
        }
        Ok(MachineRemoved {})
    }

    /// Remove this Machine: clean managed containers, persist reset, and
    /// best-effort delete the replicated row.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ClusterStoreUnavailable`] when the Cluster store is
    /// missing, [`Error::DockerUnavailable`] when Docker is missing,
    /// [`Error::Cleanup`] when managed container removal fails,
    /// [`Error::LockPoisoned`] when the local record lock is poisoned, and
    /// [`Error::Store`] when reset cannot be prepared or committed.
    pub async fn remove_local(
        &self,
        request: RemoveLocalMachineRequest,
    ) -> Result<LocalMachineRemoved, Error> {
        let machine_id = self.record()?.id();
        let replicated = self.replicated()?;
        let containers = self.containers.as_ref().ok_or(Error::DockerUnavailable)?;
        let publication = replicated.machine_publication().await;
        let prepared_reset = {
            let store = self.lock_store()?;
            if store.record().phase() == LocalMachinePhase::Resetting {
                None
            } else {
                Some(store.prepare_reset()?)
            }
        };
        if let Err(error) = containers.remove_all_managed().await {
            return Err(Error::Cleanup(error.to_string()));
        }
        if let Some(prepared_reset) = prepared_reset {
            let mut store = self.lock_store()?;
            prepared_reset.commit(&mut store)?;
        }
        let reset_warning = publication
            .remove(&machine_id)
            .await
            .err()
            .map(|error| error.to_string());
        Ok(local_removal_response(
            &self.restart,
            reset_warning,
            request.restart_on_cleanup_failure,
        ))
    }

    /// Begin a Local Machine Phase reset and request daemon restart.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Docker`] when managed container cleanup fails,
    /// [`Error::LockPoisoned`] when the local record lock is poisoned, and
    /// [`Error::Store`] when reset is not legal in the current phase.
    pub async fn reset(&self) -> Result<ResetAccepted, Error> {
        if let Some(containers) = &self.containers {
            containers.remove_all_managed().await?;
        }
        self.lock_store()?.begin_reset()?;
        self.restart.send_replace(true);
        Ok(ResetAccepted {})
    }
}

async fn machine_rtts(
    admin: &AdminClient,
    replicated: &ReplicatedStore,
) -> Result<Vec<RttObservation>, crate::corrosion::Error> {
    let machines = replicated.machines().await?.observations;
    let identities = unique_identities(machines.into_iter().map(|machine| {
        (
            IpAddr::V6(machine.management_address.0),
            MachineIdentity {
                id: machine.id,
                name: machine.name,
            },
        )
    }));
    Ok(admin
        .member_rtts()
        .await?
        .into_iter()
        .map(|mut observation| {
            observation.machine = identities.get(&observation.address.ip()).cloned();
            observation
        })
        .collect())
}

/// Read membership and RTT from the local admin socket.
///
/// Both reads must succeed; otherwise Watch keeps replicated rows.
async fn read_admin(admin: &AdminClient) -> Option<(Vec<MembershipState>, Vec<RttObservation>)> {
    let Ok((states, rtts)) = tokio::try_join!(admin.membership_states(), admin.member_rtts())
    else {
        return None;
    };
    Some((states, rtts))
}

fn rtts_by_machine(
    machines: &[Machine],
    rtts: &[RttObservation],
) -> BTreeMap<MachineId, RttStatistics> {
    let identities = unique_identities(
        machines
            .iter()
            .map(|machine| (IpAddr::V6(machine.management_address.0), machine.id)),
    );
    rtts.iter()
        .filter_map(|observation| {
            identities
                .get(&observation.address.ip())
                .copied()
                .map(|id| (id, observation.statistics.clone()))
        })
        .collect()
}

fn membership_states_by_address(
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

/// Associate one value per management address; duplicate addresses are dropped.
fn unique_identities<T>(entries: impl IntoIterator<Item = (IpAddr, T)>) -> BTreeMap<IpAddr, T> {
    let mut identities = BTreeMap::new();
    let mut ambiguous = BTreeSet::new();
    for (address, identity) in entries {
        if identities.insert(address, identity).is_some() {
            ambiguous.insert(address);
        }
    }
    identities.retain(|address, _| !ambiguous.contains(address));
    identities
}

fn local_removal_response(
    restart: &watch::Sender<bool>,
    reset_warning: Option<String>,
    restart_on_warning: bool,
) -> LocalMachineRemoved {
    if reset_warning.is_none() || restart_on_warning {
        restart.send_replace(true);
    }
    LocalMachineRemoved { reset_warning }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        net::Ipv6Addr,
        sync::{Arc, Mutex},
    };

    use super::{
        Error, LocalMachine, REGISTER_FORWARDED_METADATA, RuntimeWatchTelemetry,
        local_removal_response, read_admin, unique_identities,
    };
    use crate::{
        corrosion::{AdminClient, ReplicatedStore},
        machine::LocalMachineStore,
        rpc::MachineService,
    };
    use axum::{Router, body::Bytes, extract::State, routing::post};
    use ployz_core::{
        AdvertisedEndpoint, Machine, MachineId, MachineIdentity, MachineName, MachineRpc,
        MachineRpcServer, MachineRuntime, ManagementAddress, MembershipObservation,
        RegisterRequest, RttObservation, RttStatistics, SelectedEndpoint, WireGuardPublicKey,
    };
    use serde::Deserialize;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, transport::Server};

    const ENTRY_ID: &str = "0123456789abcdef0123456789abcdef";
    const PEER_ID: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn failed_local_removal_keeps_the_daemon_available_for_entry_fallback() {
        let (restart, restart_rx) = tokio::sync::watch::channel(false);
        let removed =
            local_removal_response(&restart, Some("replicated delete failed".into()), false);

        assert_eq!(
            removed.reset_warning.as_deref(),
            Some("replicated delete failed")
        );
        assert!(!*restart_rx.borrow());
    }

    #[test]
    fn failed_remote_removal_restarts_after_delegating_entry_fallback() {
        let (restart, restart_rx) = tokio::sync::watch::channel(false);
        local_removal_response(&restart, Some("replicated delete failed".into()), true);

        assert!(*restart_rx.borrow());
    }

    #[test]
    fn duplicate_management_addresses_have_no_identity_winner() {
        let duplicate = "192.0.2.1".parse().unwrap();
        let unique = "192.0.2.2".parse().unwrap();
        let identity = |seed: char| MachineIdentity {
            id: MachineId::parse(seed.to_string().repeat(32)).unwrap(),
            name: MachineName::parse(seed.to_string()).unwrap(),
        };
        let identities = unique_identities([
            (duplicate, identity('1')),
            (duplicate, identity('2')),
            (unique, identity('3')),
        ]);

        assert!(!identities.contains_key(&duplicate));
        assert_eq!(identities.get(&unique), Some(&identity('3')));
    }

    #[test]
    fn overlay_marks_entry_up_and_maps_inspect_rtt() {
        let entry = machine("edge", ENTRY_ID, 1);
        let peer = machine("peer", PEER_ID, 2);
        let endpoint = SelectedEndpoint("203.0.113.10:51820".parse().unwrap());
        let rtt = RttStatistics {
            median_ns: 1_500_000,
            population_stddev_ns: 250_000,
        };
        let observations = RuntimeWatchTelemetry {
            states: BTreeMap::from([(peer.management_address, MembershipObservation::Suspect)]),
            selected_endpoints: BTreeMap::from([(entry.id, endpoint)]),
            rtts: vec![RttObservation {
                peer_id: "peer".into(),
                address: format!("[{}]:51001", peer.management_address.0)
                    .parse()
                    .unwrap(),
                machine: None,
                statistics: rtt.clone(),
            }],
        }
        .overlay(vec![entry.clone(), peer.clone()], &entry.id);

        let [entry_row, peer_row] = observations.as_slice() else {
            panic!("expected entry and peer observations");
        };
        assert_eq!(entry_row.membership, MembershipObservation::Up);
        assert_eq!(entry_row.selected_endpoint, Some(endpoint));
        assert_eq!(peer_row.membership, MembershipObservation::Suspect);
        assert_eq!(peer_row.rtt, Some(rtt));
    }

    #[test]
    fn overlay_uses_down_for_peers_missing_from_admin() {
        let entry = machine("edge", ENTRY_ID, 1);
        let peer = machine("peer", PEER_ID, 2);
        let observations =
            RuntimeWatchTelemetry::default().overlay(vec![entry.clone(), peer.clone()], &entry.id);

        let [entry_row, peer_row] = observations.as_slice() else {
            panic!("expected entry and peer observations");
        };
        assert_eq!(entry_row.membership, MembershipObservation::Up);
        assert_eq!(peer_row.membership, MembershipObservation::Down);
        assert_eq!(peer_row.rtt, None);
    }

    #[tokio::test]
    async fn missing_admin_socket_is_unavailable_telemetry() {
        let admin = AdminClient::new("/no/such/ployz-admin.sock");
        assert!(read_admin(&admin).await.is_none());
    }

    #[tokio::test]
    async fn named_allocator_admits_register_locally() {
        let (dir, store, founder) = participating("ployzd-register-local");
        let replica = ClusterReplica::start(Some(founder.id), vec![founder.clone()]).await;
        let local =
            LocalMachine::new(store, tokio::sync::watch::channel(false).0).with_cluster(Some((
                replica.store.clone(),
                AdminClient::new("/no/such-admin.sock"),
            )));

        let registered = local.register(joiner_request()).await.unwrap();

        assert_eq!(registered.assigned_machine.name.as_str(), "joiner");
        assert_eq!(
            registered.assigned_machine.subnet.to_string(),
            "10.210.1.0/24"
        );
        assert!(
            replica
                .machines
                .lock()
                .unwrap()
                .iter()
                .any(|machine| machine.name.as_str() == "joiner")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn missing_allocator_row_does_not_admit() {
        let (dir, store, founder) = participating("ployzd-register-missing");
        let replica = ClusterReplica::start(None, vec![founder]).await;
        let local =
            LocalMachine::new(store, tokio::sync::watch::channel(false).0).with_cluster(Some((
                replica.store.clone(),
                AdminClient::new("/no/such-admin.sock"),
            )));

        let error = local.register(joiner_request()).await.unwrap_err();

        assert!(matches!(error, Error::NotAllocator));
        assert!(
            !replica
                .machines
                .lock()
                .unwrap()
                .iter()
                .any(|machine| machine.name.as_str() == "joiner")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn contact_forwards_register_and_returns_the_allocator_payload() {
        let (allocator_dir, allocator_store, mut allocator_machine) =
            participating("ployzd-register-allocator");
        let listener = TcpListener::bind("[::1]:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        allocator_machine.management_address = ManagementAddress(Ipv6Addr::LOCALHOST);
        let allocator_replica =
            ClusterReplica::start(Some(allocator_machine.id), vec![allocator_machine.clone()])
                .await;
        let allocator = MachineService::with_cluster(
            allocator_store,
            tokio::sync::watch::channel(false).0,
            Some((
                allocator_replica.store.clone(),
                AdminClient::new("/no/such-admin.sock"),
            )),
        );
        tokio::spawn(
            Server::builder()
                .add_service(MachineRpcServer::new(allocator))
                .serve_with_incoming(TcpListenerStream::new(listener)),
        );

        let (contact_dir, contact_store, _contact) = participating("ployzd-register-contact");
        let contact_replica =
            ClusterReplica::start(Some(allocator_machine.id), vec![allocator_machine.clone()])
                .await;
        let contact = LocalMachine::new(contact_store, tokio::sync::watch::channel(false).0)
            .with_cluster(Some((
                contact_replica.store.clone(),
                AdminClient::new("/no/such-admin.sock"),
            )))
            .with_machine_api_port(port);

        let registered = contact.register(joiner_request()).await.unwrap();

        assert_eq!(registered.assigned_machine.name.as_str(), "joiner");
        assert_eq!(
            registered.assigned_machine.subnet.to_string(),
            "10.210.1.0/24"
        );
        assert!(
            allocator_replica
                .machines
                .lock()
                .unwrap()
                .iter()
                .any(|machine| machine.name.as_str() == "joiner"),
            "Allocator admits locally"
        );
        assert!(
            !contact_replica
                .machines
                .lock()
                .unwrap()
                .iter()
                .any(|machine| machine.name.as_str() == "joiner"),
            "contact must not allocate locally"
        );
        let _ = std::fs::remove_dir_all(allocator_dir);
        let _ = std::fs::remove_dir_all(contact_dir);
    }

    #[tokio::test]
    async fn forwarded_register_does_not_admit_or_forward_when_kv_names_another_machine() {
        let (dir, store, local_machine) = participating("ployzd-register-one-hop");
        let other = MachineId::parse("b".repeat(32)).unwrap();
        let mut named = local_machine.clone();
        named.id = other;
        named.management_address = ManagementAddress(Ipv6Addr::LOCALHOST);
        let replica = ClusterReplica::start(Some(other), vec![named]).await;
        let local = LocalMachine::new(store, tokio::sync::watch::channel(false).0)
            .with_cluster(Some((
                replica.store.clone(),
                AdminClient::new("/no/such-admin.sock"),
            )))
            .with_machine_api_port(1);

        let error = local.admit_register(joiner_request()).await.unwrap_err();

        assert!(matches!(error, Error::NotAllocator));
        assert!(
            !replica
                .machines
                .lock()
                .unwrap()
                .iter()
                .any(|machine| machine.name.as_str() == "joiner")
        );
        assert!(
            !replica
                .executes
                .lock()
                .unwrap()
                .iter()
                .any(|query| query.contains("allocator")),
            "no steal"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn unreachable_allocator_does_not_steal() {
        let (dir, store, _local_machine) = participating("ployzd-register-unreachable");
        let allocator_id = MachineId::parse("c".repeat(32)).unwrap();
        let named = Machine {
            id: allocator_id,
            name: MachineName::parse("allocator").unwrap(),
            subnet: "10.210.0.0/24".parse().unwrap(),
            management_address: ManagementAddress(Ipv6Addr::LOCALHOST),
            public_key: WireGuardPublicKey([3; 32]),
            public_ip: None,
            advertised_endpoints: vec![AdvertisedEndpoint("192.0.2.3:51820".parse().unwrap())],
            runtime: MachineRuntime::default(),
        };
        let replica = ClusterReplica::start(Some(allocator_id), vec![named]).await;
        let local = LocalMachine::new(store, tokio::sync::watch::channel(false).0)
            .with_cluster(Some((
                replica.store.clone(),
                AdminClient::new("/no/such-admin.sock"),
            )))
            .with_machine_api_port(1);

        let error = local.register(joiner_request()).await.unwrap_err();

        assert!(matches!(error, Error::AllocatorUnreachable));
        assert!(
            !replica
                .executes
                .lock()
                .unwrap()
                .iter()
                .any(|query| query.contains("allocator")),
            "no steal"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn forwarded_rpc_metadata_admits_locally_only() {
        let (dir, store, founder) = participating("ployzd-register-metadata");
        let replica = ClusterReplica::start(Some(founder.id), vec![founder.clone()]).await;
        let service = MachineService::with_cluster(
            store,
            tokio::sync::watch::channel(false).0,
            Some((
                replica.store.clone(),
                AdminClient::new("/no/such-admin.sock"),
            )),
        );
        let mut request = Request::new(
            ployz_core::op::Register::into_request(joiner_request())
                .encode()
                .unwrap(),
        );
        request.metadata_mut().insert(
            REGISTER_FORWARDED_METADATA,
            "1".parse().expect("ASCII metadata"),
        );

        let registered = service
            .register(request)
            .await
            .unwrap()
            .into_inner()
            .decode_response()
            .unwrap()
            .decode::<ployz_core::op::Register>()
            .unwrap();

        assert_eq!(registered.assigned_machine.name.as_str(), "joiner");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn participating(prefix: &str) -> (std::path::PathBuf, Arc<Mutex<LocalMachineStore>>, Machine) {
        let dir = std::env::temp_dir().join(format!("{prefix}-{}", MachineId::random()));
        let mut store = LocalMachineStore::open(&dir).unwrap();
        let machine = store
            .initialize(
                MachineName::parse("edge").unwrap(),
                "10.210.0.0/16".parse().unwrap(),
                None,
                vec![AdvertisedEndpoint("192.0.2.1:51820".parse().unwrap())],
                None,
            )
            .unwrap();
        (dir, Arc::new(Mutex::new(store)), machine)
    }

    fn joiner_request() -> RegisterRequest {
        RegisterRequest {
            name: MachineName::parse("joiner").unwrap(),
            public_key: WireGuardPublicKey([7; 32]),
            public_ip: None,
            advertised_endpoints: vec![AdvertisedEndpoint("192.0.2.9:51820".parse().unwrap())],
            runtime: MachineRuntime::default(),
        }
    }

    #[derive(Clone)]
    struct ClusterReplica {
        store: ReplicatedStore,
        allocator: Arc<Mutex<Option<MachineId>>>,
        machines: Arc<Mutex<Vec<Machine>>>,
        executes: Arc<Mutex<Vec<String>>>,
    }

    impl ClusterReplica {
        async fn start(allocator: Option<MachineId>, machines: Vec<Machine>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let replica = Self {
                store: ReplicatedStore::http1(address),
                allocator: Arc::new(Mutex::new(allocator)),
                machines: Arc::new(Mutex::new(machines)),
                executes: Arc::new(Mutex::new(Vec::new())),
            };
            let serving = replica.clone();
            tokio::spawn(async move {
                axum::serve(
                    listener,
                    Router::new()
                        .route("/v1/queries", post(cluster_query))
                        .route("/v1/transactions", post(cluster_execute))
                        .with_state(serving),
                )
                .await
                .unwrap();
            });
            replica
        }
    }

    #[derive(Deserialize)]
    struct ClusterStatement {
        query: String,
        #[serde(default)]
        params: Vec<Value>,
    }

    async fn cluster_query(State(replica): State<ClusterReplica>, body: Bytes) -> Vec<u8> {
        let statement: ClusterStatement = serde_json::from_slice(&body).unwrap();
        let query = statement.query.as_str();
        if query.contains("crsql_db_versions") {
            return query_body(&["site_id", "db_version"], Vec::new());
        }
        if query.contains("key = 'allocator'") {
            let allocator = replica.allocator.lock().unwrap();
            return match &*allocator {
                Some(id) => query_body(&["value"], vec![vec![json!(id.as_str())]]),
                None => query_body(&["value"], Vec::new()),
            };
        }
        if query.contains("key = 'network'") {
            return query_body(&["value"], vec![vec![json!("10.210.0.0/16")]]);
        }
        if query.contains("FROM machines WHERE id") {
            let id = statement.params.first().and_then(Value::as_str).unwrap();
            let machines = replica.machines.lock().unwrap();
            return match machines.iter().find(|machine| machine.id.as_str() == id) {
                Some(machine) => query_body(
                    &["info"],
                    vec![vec![json!(serde_json::to_string(machine).unwrap())]],
                ),
                None => query_body(&["info"], Vec::new()),
            };
        }
        if query.contains("FROM machines") {
            let machines = replica.machines.lock().unwrap();
            let rows = machines
                .iter()
                .map(|machine| {
                    vec![
                        json!(machine.id.as_str()),
                        json!(serde_json::to_string(machine).unwrap()),
                    ]
                })
                .collect();
            return query_body(&["id", "info"], rows);
        }
        query_body(&["value"], Vec::new())
    }

    async fn cluster_execute(State(replica): State<ClusterReplica>, body: Bytes) -> String {
        let statements: Vec<ClusterStatement> = serde_json::from_slice(&body).unwrap();
        for statement in statements {
            replica
                .executes
                .lock()
                .unwrap()
                .push(statement.query.clone());
            if statement.query.contains("INSERT INTO machines") {
                let info = statement.params.get(1).and_then(Value::as_str).unwrap();
                let machine: Machine = serde_json::from_str(info).unwrap();
                replica.machines.lock().unwrap().push(machine);
            }
        }
        serde_json::to_string(&json!({
            "results": [{"rows_affected": 1, "time": 0.0}],
            "time": 0.0
        }))
        .unwrap()
    }

    fn query_body(columns: &[&str], rows: Vec<Vec<Value>>) -> Vec<u8> {
        let mut body = serde_json::to_vec(&json!({ "columns": columns })).unwrap();
        for (index, row) in rows.into_iter().enumerate() {
            body.extend(serde_json::to_vec(&json!({ "row": [index, row] })).unwrap());
        }
        body.extend(serde_json::to_vec(&json!({ "eoq": { "time": 0.0 } })).unwrap());
        body
    }

    fn machine(name: &str, id: &str, seed: u8) -> Machine {
        Machine {
            id: MachineId::parse(id).unwrap(),
            name: MachineName::parse(name).unwrap(),
            subnet: format!("10.210.{seed}.0/24").parse().unwrap(),
            management_address: ManagementAddress(format!("fdcc::{seed}").parse().unwrap()),
            public_key: WireGuardPublicKey([seed; 32]),
            public_ip: None,
            advertised_endpoints: vec![AdvertisedEndpoint(
                format!("203.0.113.{seed}:51820").parse().unwrap(),
            )],
            runtime: MachineRuntime::default(),
        }
    }
}
