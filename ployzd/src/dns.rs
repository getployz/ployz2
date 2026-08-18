use std::{
    collections::{HashMap, hash_map::Entry},
    fs, io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, RwLock},
    time::Duration,
};

use async_trait::async_trait;
use hickory_server::proto::{
    op::{Header, HeaderCounts, Message, Metadata, ResponseCode},
    rr::{Name, RData, Record, RecordType, rdata::A},
};
use hickory_server::{
    Server,
    net::{runtime::Time, xfer::Protocol},
    server::{Request, RequestHandler, ResponseHandler, ResponseInfo},
    zone_handler::MessageResponseBuilder,
};
use ipnet::Ipv4Net;
use ployz_core::{
    ContainerObservation, Machine, ProjectName, QualifiedService, ServiceId, ServiceName,
    service_containers, serving_replicas,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
};
use tokio_util::sync::CancellationToken;

use crate::corrosion::{ReplicatedStore, Subscription};

mod query;

use query::{InternalQuery, MachineServiceNameTarget, MachineServiceTarget, Query, Target, parse};

pub const PORT: u16 = 53;
const FORWARD_TIMEOUT: Duration = Duration::from_secs(3);
const TCP_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const TCP_RESPONSE_BUFFER: usize = 32;

#[derive(Debug, PartialEq)]
enum ResponsePlan {
    Forward,
    Internal {
        code: ResponseCode,
        answers: Vec<Record>,
    },
}

/// ID-index lookup. [`Known`] empty is NXDOMAIN; [`Unknown`] falls back to names.
enum ServiceIdIndex<'index> {
    Known(&'index [Ipv4Addr]),
    Unknown,
}

/// Observer-relative Caller Project from a source Container Address.
enum CallerProject {
    Unique(ProjectName),
    Ambiguous,
}

struct Projection {
    service_ids: HashMap<ServiceId, Vec<Ipv4Addr>>,
    identities: HashMap<QualifiedService, Vec<Ipv4Addr>>,
    machine_identities: HashMap<MachineServiceTarget, Vec<Ipv4Addr>>,
    callers: HashMap<Ipv4Addr, CallerProject>,
}

impl Projection {
    fn from_observations(observations: &[ContainerObservation]) -> Self {
        // TODO(UT-117, UT-118): keep Membership Observations out of DNS projection until a
        // product decision replaces the baseline's deliberately membership-blind behavior.
        let containers = service_containers(observations.iter().cloned());
        let mut service_ids = HashMap::<ServiceId, Vec<Ipv4Addr>>::new();
        let mut identities = HashMap::<QualifiedService, Vec<Ipv4Addr>>::new();
        let mut machine_identities = HashMap::<MachineServiceTarget, Vec<Ipv4Addr>>::new();
        let mut callers = HashMap::<Ipv4Addr, CallerProject>::new();
        // Known IDs enter the index even with no serving addresses so empty ≠ missing.
        // Caller matching uses every addressed Service Container, not Serving Containers.
        for container in &containers {
            let observation = container.as_observation();
            service_ids.entry(observation.service_id).or_default();
            let Some(address) = observation.address else {
                continue;
            };
            match callers.entry(address.0) {
                Entry::Vacant(entry) => {
                    entry.insert(CallerProject::Unique(observation.project_name.clone()));
                }
                Entry::Occupied(mut entry) => {
                    *entry.get_mut() = CallerProject::Ambiguous;
                }
            }
        }
        for container in serving_replicas(&containers) {
            let observation = container.as_observation();
            let address = observation
                .address
                .expect("Serving Container has a Container Address");
            let identity = observation.identity();
            service_ids
                .entry(observation.service_id)
                .or_default()
                .push(address.0);
            identities
                .entry(identity.clone())
                .or_default()
                .push(address.0);
            machine_identities
                .entry(MachineServiceTarget {
                    machine_id: observation.machine_id,
                    identity,
                })
                .or_default()
                .push(address.0);
        }
        Self {
            service_ids,
            identities,
            machine_identities,
            callers,
        }
    }

    fn plan(
        &self,
        name: &Name,
        record_type: RecordType,
        local_subnet: Ipv4Net,
        source: IpAddr,
    ) -> ResponsePlan {
        match parse(name) {
            Query::Forward => ResponsePlan::Forward,
            Query::Internal(query) => {
                self.plan_internal(name, record_type, local_subnet, source, query)
            }
        }
    }

    fn plan_internal(
        &self,
        name: &Name,
        record_type: RecordType,
        local_subnet: Ipv4Net,
        source: IpAddr,
        query: InternalQuery,
    ) -> ResponsePlan {
        if record_type != RecordType::A {
            // TODO(UT-110): internal records remain A-only; other types return an authoritative
            // empty NOERROR response until a product decision adds them.
            return ResponsePlan::Internal {
                code: ResponseCode::NoError,
                answers: Vec::new(),
            };
        }
        let mut addresses = self.addresses(&query.target, self.caller_project(source));
        if query.nearest {
            addresses.sort_by_key(|address| !local_subnet.contains(address));
        }
        if addresses.is_empty() {
            return ResponsePlan::Internal {
                code: ResponseCode::NXDomain,
                answers: Vec::new(),
            };
        }
        // TODO(UT-111): keep TTL zero rather than adding a DNS cache without a product decision.
        let answers = addresses
            .into_iter()
            .map(|address| Record::from_rdata(name.clone(), 0, RData::A(A(address))))
            .collect();
        ResponsePlan::Internal {
            code: ResponseCode::NoError,
            answers,
        }
    }

    fn addresses(&self, target: &Target, caller: Option<&ProjectName>) -> Vec<Ipv4Addr> {
        match target {
            Target::Empty => Vec::new(),
            Target::ServiceId(id) => match self.service_id_index(id) {
                ServiceIdIndex::Known(addresses) => addresses.to_vec(),
                ServiceIdIndex::Unknown => self.unique_name_addresses(
                    &ServiceName::parse(id.as_str())
                        .expect("a Service ID is a DNS-label Service Name"),
                ),
            },
            Target::Identity(identity) => {
                self.identities.get(identity).cloned().unwrap_or_default()
            }
            Target::ServiceName(name) => {
                let Some(project) = caller else {
                    return Vec::new();
                };
                self.identities
                    .get(&QualifiedService::new(project.clone(), name.clone()))
                    .cloned()
                    .unwrap_or_default()
            }
            Target::MachineIdentity(target) => self
                .machine_identities
                .get(target)
                .cloned()
                .unwrap_or_default(),
            Target::MachineServiceName(target) => self.unique_machine_name_addresses(target),
        }
    }

    fn caller_project(&self, source: IpAddr) -> Option<&ProjectName> {
        let IpAddr::V4(source) = source else {
            return None;
        };
        match self.callers.get(&source) {
            Some(CallerProject::Unique(project)) => Some(project),
            Some(CallerProject::Ambiguous) | None => None,
        }
    }

    fn unique_name_addresses(&self, name: &ServiceName) -> Vec<Ipv4Addr> {
        let mut matches = self
            .identities
            .iter()
            .filter(|(identity, _)| &identity.name == name);
        match (matches.next(), matches.next()) {
            (Some((_, addresses)), None) => addresses.clone(),
            _ => Vec::new(),
        }
    }

    fn unique_machine_name_addresses(&self, target: &MachineServiceNameTarget) -> Vec<Ipv4Addr> {
        let mut matches = self.machine_identities.iter().filter(|(key, _)| {
            key.machine_id == target.machine_id && key.identity.name == target.service_name
        });
        match (matches.next(), matches.next()) {
            (Some((_, addresses)), None) => addresses.clone(),
            _ => Vec::new(),
        }
    }

    fn service_id_index(&self, id: &ServiceId) -> ServiceIdIndex<'_> {
        match self.service_ids.get(id) {
            Some(addresses) => ServiceIdIndex::Known(addresses),
            None => ServiceIdIndex::Unknown,
        }
    }
}

struct Handler {
    projection: Arc<RwLock<Projection>>,
    local_subnet: Ipv4Net,
    upstreams: Vec<SocketAddr>,
}

#[async_trait]
impl RequestHandler for Handler {
    async fn handle_request<R: ResponseHandler, T: Time>(
        &self,
        request: &Request,
        mut response_handle: R,
    ) -> ResponseInfo {
        let Ok(info) = request.request_info() else {
            return send_error(request, response_handle, ResponseCode::FormErr).await;
        };
        let plan = self
            .projection
            .read()
            .map(|projection| {
                projection.plan(
                    info.query.original().name(),
                    info.query.query_type(),
                    self.local_subnet,
                    info.src.ip(),
                )
            })
            .unwrap_or(ResponsePlan::Internal {
                code: ResponseCode::ServFail,
                answers: Vec::new(),
            });
        match plan {
            ResponsePlan::Internal { code, answers } => {
                let mut metadata = Metadata::response_from_request(&request.metadata);
                metadata.authoritative = true;
                metadata.recursion_available = true;
                metadata.response_code = code;
                let response = MessageResponseBuilder::from_message_request(request).build(
                    metadata,
                    answers.iter(),
                    [].iter(),
                    [].iter(),
                    [].iter(),
                );
                response_handle
                    .send_response(response)
                    .await
                    .unwrap_or_else(|_| failed_response(request, ResponseCode::ServFail))
            }
            ResponsePlan::Forward => match self.forward(request).await {
                Ok(message) => {
                    let mut builder = MessageResponseBuilder::from_message_request(request);
                    if let Some(edns) = message.edns.as_ref() {
                        builder.edns(edns);
                    }
                    let response = builder.build(
                        message.metadata,
                        message.answers.iter(),
                        message.authorities.iter(),
                        [].iter(),
                        message.additionals.iter(),
                    );
                    response_handle
                        .send_response(response)
                        .await
                        .unwrap_or_else(|_| failed_response(request, ResponseCode::ServFail))
                }
                Err(error) => {
                    eprintln!("failed to forward DNS query: {error}");
                    send_error(request, response_handle, ResponseCode::ServFail).await
                }
            },
        }
    }
}

impl Handler {
    async fn forward(&self, request: &Request) -> io::Result<Message> {
        let mut last_error = io::Error::other("no upstream DNS servers configured");
        for upstream in &self.upstreams {
            let result = tokio::time::timeout(FORWARD_TIMEOUT, async {
                match request.protocol() {
                    Protocol::Udp => forward_udp(request.as_slice(), *upstream).await,
                    Protocol::Tcp => forward_tcp(request.as_slice(), *upstream).await,
                    protocol => Err(io::Error::other(format!(
                        "unsupported forwarding transport {protocol:?}"
                    ))),
                }
            })
            .await;
            match result {
                Ok(Ok(message)) => return Ok(message),
                Ok(Err(error)) => last_error = error,
                Err(_) => {
                    last_error = io::Error::new(io::ErrorKind::TimedOut, "DNS upstream timed out")
                }
            }
        }
        Err(last_error)
    }
}

pub async fn run(
    machine: Machine,
    replicated: ReplicatedStore,
    upstreams: Option<Vec<SocketAddr>>,
    shutdown: CancellationToken,
) -> io::Result<()> {
    let gateway = machine.subnet.gateway().0;
    let listen_address = SocketAddr::new(IpAddr::V4(gateway), PORT);
    let mut changes = replicated
        .subscribe_container_changes()
        .await
        .map_err(io::Error::other)?;
    let observations = replicated.containers().await.map_err(io::Error::other)?;
    let projection = Arc::new(RwLock::new(Projection::from_observations(
        &observations.observations,
    )));
    let handler = Handler {
        projection: Arc::clone(&projection),
        local_subnet: machine.subnet.into(),
        upstreams: configured_upstreams(upstreams, gateway),
    };
    let udp = UdpSocket::bind(listen_address).await?;
    let tcp = match TcpListener::bind(listen_address).await {
        Ok(listener) => Some(listener),
        Err(error) => {
            eprintln!("failed to bind best-effort DNS TCP listener on {listen_address}: {error}");
            None
        }
    };
    let mut server = Server::new(handler);
    server.register_socket(udp);
    if let Some(tcp) = tcp {
        server.register_listener(tcp, TCP_REQUEST_TIMEOUT, TCP_RESPONSE_BUFFER);
    }
    let server = run_server(server, shutdown.clone());
    let projection = watch_projection(replicated, projection, &mut changes, shutdown);
    tokio::try_join!(server, projection).map(|_| ())
}

async fn run_server(mut server: Server<Handler>, shutdown: CancellationToken) -> io::Result<()> {
    tokio::select! {
        result = server.block_until_done() => result.map_err(io::Error::other),
        () = shutdown.cancelled() => {
            server.shutdown_gracefully().await.map_err(io::Error::other)
        }
    }
}

async fn watch_projection(
    replicated: ReplicatedStore,
    projection: Arc<RwLock<Projection>>,
    changes: &mut Subscription,
    shutdown: CancellationToken,
) -> io::Result<()> {
    loop {
        tokio::select! {
            changed = changes.changed() => {
                changed.map_err(io::Error::other)?;
                match replicated.containers().await {
                    Ok(observations) => {
                        *projection
                            .write()
                            .map_err(|_| io::Error::other("DNS projection lock poisoned"))? =
                            Projection::from_observations(&observations.observations);
                    }
                    Err(error) => eprintln!("failed to rebuild DNS projection: {error}"),
                }
            }
            () = shutdown.cancelled() => return Ok(()),
        }
    }
}

fn configured_upstreams(
    upstreams: Option<Vec<SocketAddr>>,
    listen_address: Ipv4Addr,
) -> Vec<SocketAddr> {
    upstreams.unwrap_or_else(|| system_upstreams(listen_address))
}

fn system_upstreams(listen_address: Ipv4Addr) -> Vec<SocketAddr> {
    match fs::read_to_string("/etc/resolv.conf") {
        Ok(text) => nameservers_from_resolv_conf(&text, listen_address),
        Err(error) => {
            eprintln!("failed to load DNS upstreams from /etc/resolv.conf: {error}");
            Vec::new()
        }
    }
}

fn nameservers_from_resolv_conf(text: &str, listen: Ipv4Addr) -> Vec<SocketAddr> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            if words.next() != Some("nameserver") {
                return None;
            }
            let ip: IpAddr = words.next()?.parse().ok()?;
            (ip != IpAddr::V4(listen)).then_some(SocketAddr::new(ip, PORT))
        })
        .collect()
}

async fn forward_udp(request: &[u8], upstream: SocketAddr) -> io::Result<Message> {
    let bind = if upstream.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind).await?;
    socket.connect(upstream).await?;
    socket.send(request).await?;
    let mut response = vec![0; u16::MAX as usize];
    let length = socket.recv(&mut response).await?;
    response.truncate(length);
    decode_message(&response)
}

async fn forward_tcp(request: &[u8], upstream: SocketAddr) -> io::Result<Message> {
    let length = u16::try_from(request.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "DNS request exceeds TCP framing",
        )
    })?;
    let mut stream = TcpStream::connect(upstream).await?;
    stream.write_u16(length).await?;
    stream.write_all(request).await?;
    let length = stream.read_u16().await?;
    let mut response = vec![0; usize::from(length)];
    stream.read_exact(&mut response).await?;
    decode_message(&response)
}

fn decode_message(bytes: &[u8]) -> io::Result<Message> {
    Message::from_vec(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

async fn send_error<R: ResponseHandler>(
    request: &Request,
    mut response_handle: R,
    code: ResponseCode,
) -> ResponseInfo {
    let response =
        MessageResponseBuilder::from_message_request(request).error_msg(&request.metadata, code);
    response_handle
        .send_response(response)
        .await
        .unwrap_or_else(|_| failed_response(request, code))
}

fn failed_response(request: &Request, code: ResponseCode) -> ResponseInfo {
    let mut metadata = Metadata::response_from_request(&request.metadata);
    metadata.response_code = code;
    Header {
        metadata,
        counts: HeaderCounts::default(),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ployz_core::{
        ContainerAddress, ContainerId, ContainerKind, ContainerRuntimeObservation,
        HealthObservation, MachineId, ProjectName, ResolvedServiceSpec, ServiceId, ServiceName,
    };
    use serde_json::json;

    use super::*;

    const HOST: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));

    #[test]
    fn does_not_publish_hook_container_addresses() {
        let machine = MachineId::parse("a".repeat(32)).unwrap();
        let service = ServiceId::parse("b".repeat(32)).unwrap();
        let name = ServiceName::parse("api").unwrap();
        let observations = [
            observation(
                1,
                &machine,
                &service,
                &name,
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 2]),
            ),
            observation(
                2,
                &machine,
                &service,
                &name,
                ContainerKind::PreDeployHook,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 4]),
            ),
        ];

        assert_eq!(
            addresses(Projection::from_observations(&observations).plan(
                &Name::from_ascii("api.app.internal.").unwrap(),
                RecordType::A,
                "10.210.1.0/24".parse().unwrap(),
                HOST,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 2)]
        );
    }

    #[test]
    fn projects_only_healthy_addressed_service_containers() {
        let machine = MachineId::parse("a".repeat(32)).unwrap();
        let service = ServiceId::parse("b".repeat(32)).unwrap();
        let name = ServiceName::parse("api").unwrap();
        let mut observations = vec![
            observation(
                1,
                &machine,
                &service,
                &name,
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 2]),
            ),
            observation(
                2,
                &machine,
                &service,
                &name,
                ContainerKind::ServiceContainer,
                running(HealthObservation::NotConfigured),
                Some([10, 210, 1, 3]),
            ),
        ];
        for (suffix, kind, runtime, address) in [
            (
                3,
                ContainerKind::PreDeployHook,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 4]),
            ),
            (
                4,
                ContainerKind::ServiceContainer,
                running(HealthObservation::Starting),
                Some([10, 210, 1, 5]),
            ),
            (
                5,
                ContainerKind::ServiceContainer,
                running(HealthObservation::Unhealthy),
                Some([10, 210, 1, 6]),
            ),
            (
                6,
                ContainerKind::ServiceContainer,
                ContainerRuntimeObservation::Exited { code: 0 },
                Some([10, 210, 1, 7]),
            ),
            (
                7,
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                None,
            ),
        ] {
            observations.push(observation(
                suffix, &machine, &service, &name, kind, runtime, address,
            ));
        }

        assert_eq!(
            addresses(Projection::from_observations(&observations).plan(
                &Name::from_ascii("api.app.internal.").unwrap(),
                RecordType::A,
                "10.210.1.0/24".parse().unwrap(),
                HOST,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 2), Ipv4Addr::new(10, 210, 1, 3)]
        );
    }

    #[test]
    fn resolves_name_id_machine_and_nearest_without_dropping_duplicates() {
        let local = MachineId::parse("a".repeat(32)).unwrap();
        let remote = MachineId::parse("c".repeat(32)).unwrap();
        let first = ServiceId::parse("b".repeat(32)).unwrap();
        let second = ServiceId::parse("d".repeat(32)).unwrap();
        let name = ServiceName::parse("api").unwrap();
        let observations = vec![
            observation(
                1,
                &remote,
                &first,
                &name,
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 2, 2]),
            ),
            observation(
                2,
                &local,
                &second,
                &name,
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 2]),
            ),
        ];
        let projection = Projection::from_observations(&observations);
        let subnet = "10.210.1.0/24".parse().unwrap();
        let caller = IpAddr::V4(Ipv4Addr::new(10, 210, 1, 2));

        assert_eq!(
            address_set(projection.plan(
                &Name::from_ascii("api.internal.").unwrap(),
                RecordType::A,
                subnet,
                caller,
            )),
            [Ipv4Addr::new(10, 210, 1, 2), Ipv4Addr::new(10, 210, 2, 2)].into()
        );
        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii(format!("{first}.internal.")).unwrap(),
                RecordType::A,
                subnet,
                HOST,
            )),
            vec![Ipv4Addr::new(10, 210, 2, 2)]
        );
        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii(format!("{local}.m.api.internal.")).unwrap(),
                RecordType::A,
                subnet,
                HOST,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 2)]
        );
        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii("nearest.api.internal.").unwrap(),
                RecordType::A,
                subnet,
                caller,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 2), Ipv4Addr::new(10, 210, 2, 2)]
        );
        assert_eq!(
            address_set(projection.plan(
                &Name::from_ascii("rr.api.internal.").unwrap(),
                RecordType::A,
                subnet,
                caller,
            )),
            [Ipv4Addr::new(10, 210, 1, 2), Ipv4Addr::new(10, 210, 2, 2)].into()
        );
    }

    #[test]
    fn two_projects_keep_separate_qualified_answers() {
        let machine = MachineId::parse("a".repeat(32)).unwrap();
        let staging_id = ServiceId::parse("b".repeat(32)).unwrap();
        let prod_id = ServiceId::parse("c".repeat(32)).unwrap();
        let name = ServiceName::parse("web").unwrap();
        let mut staging = observation(
            1,
            &machine,
            &staging_id,
            &name,
            ContainerKind::ServiceContainer,
            running(HealthObservation::Healthy),
            Some([10, 210, 1, 2]),
        );
        staging.project_name = ProjectName::parse("shop-staging").unwrap();
        let mut prod = observation(
            2,
            &machine,
            &prod_id,
            &name,
            ContainerKind::ServiceContainer,
            running(HealthObservation::Healthy),
            Some([10, 210, 1, 3]),
        );
        prod.project_name = ProjectName::parse("shop-prod").unwrap();
        let projection = Projection::from_observations(&[staging, prod]);
        let subnet = "10.210.1.0/24".parse().unwrap();

        assert!(
            addresses(projection.plan(
                &Name::from_ascii("web.internal.").unwrap(),
                RecordType::A,
                subnet,
                HOST,
            ))
            .is_empty()
        );
        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii("web.shop-staging.internal.").unwrap(),
                RecordType::A,
                subnet,
                HOST,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 2)]
        );
        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii("web.shop-prod.internal.").unwrap(),
                RecordType::A,
                subnet,
                HOST,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 3)]
        );
        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii(format!("{machine}.m.web.shop-staging.internal.")).unwrap(),
                RecordType::A,
                subnet,
                HOST,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 2)]
        );
    }

    #[test]
    fn service_id_selector_takes_precedence_over_a_colliding_name() {
        let machine = MachineId::parse("a".repeat(32)).unwrap();
        let selected_id = ServiceId::parse("b".repeat(32)).unwrap();
        let other_id = ServiceId::parse("c".repeat(32)).unwrap();
        let colliding_name = ServiceName::parse(selected_id.to_string()).unwrap();
        let selected_name = ServiceName::parse("selected").unwrap();
        let projection = Projection::from_observations(&[
            observation(
                1,
                &machine,
                &selected_id,
                &selected_name,
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 2]),
            ),
            observation(
                2,
                &machine,
                &other_id,
                &colliding_name,
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 3]),
            ),
        ]);

        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii(format!("{selected_id}.internal.")).unwrap(),
                RecordType::A,
                "10.210.1.0/24".parse().unwrap(),
                HOST,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 2)]
        );
    }

    #[test]
    fn empty_service_id_selector_does_not_fall_back_to_a_colliding_name() {
        let machine = MachineId::parse("a".repeat(32)).unwrap();
        let selected_id = ServiceId::parse("b".repeat(32)).unwrap();
        let other_id = ServiceId::parse("c".repeat(32)).unwrap();
        let colliding_name = ServiceName::parse(selected_id.to_string()).unwrap();
        let projection = Projection::from_observations(&[
            observation(
                1,
                &machine,
                &selected_id,
                &ServiceName::parse("selected").unwrap(),
                ContainerKind::ServiceContainer,
                running(HealthObservation::Unhealthy),
                Some([10, 210, 1, 2]),
            ),
            observation(
                2,
                &machine,
                &other_id,
                &colliding_name,
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 3]),
            ),
        ]);

        assert!(matches!(
            projection.plan(
                &Name::from_ascii(format!("{selected_id}.internal.")).unwrap(),
                RecordType::A,
                "10.210.1.0/24".parse().unwrap(),
                HOST,
            ),
            ResponsePlan::Internal {
                code: ResponseCode::NXDomain,
                answers,
            } if answers.is_empty()
        ));
    }

    #[test]
    fn unknown_service_id_falls_back_to_the_name_index() {
        let machine = MachineId::parse("a".repeat(32)).unwrap();
        let missing_id = ServiceId::parse("b".repeat(32)).unwrap();
        let other_id = ServiceId::parse("c".repeat(32)).unwrap();
        let colliding_name = ServiceName::parse(missing_id.to_string()).unwrap();
        let projection = Projection::from_observations(&[observation(
            1,
            &machine,
            &other_id,
            &colliding_name,
            ContainerKind::ServiceContainer,
            running(HealthObservation::Healthy),
            Some([10, 210, 1, 3]),
        )]);

        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii(format!("{missing_id}.internal.")).unwrap(),
                RecordType::A,
                "10.210.1.0/24".parse().unwrap(),
                HOST,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 3)]
        );
    }

    #[test]
    fn plans_internal_protocol_responses_and_external_forwarding() {
        let projection = Projection::from_observations(&[]);
        let subnet = "10.210.1.0/24".parse().unwrap();

        assert_eq!(
            projection.plan(
                &Name::from_ascii("internal.").unwrap(),
                RecordType::A,
                subnet,
                HOST,
            ),
            ResponsePlan::Internal {
                code: ResponseCode::NXDomain,
                answers: Vec::new(),
            }
        );
        assert_eq!(
            projection.plan(
                &Name::from_ascii("missing.internal.").unwrap(),
                RecordType::A,
                subnet,
                HOST,
            ),
            ResponsePlan::Internal {
                code: ResponseCode::NXDomain,
                answers: Vec::new(),
            }
        );
        for record_type in [RecordType::AAAA, RecordType::SRV, RecordType::TXT] {
            assert_eq!(
                projection.plan(
                    &Name::from_ascii("missing.internal.").unwrap(),
                    record_type,
                    subnet,
                    HOST,
                ),
                ResponsePlan::Internal {
                    code: ResponseCode::NoError,
                    answers: Vec::new(),
                }
            );
        }
        assert_eq!(
            projection.plan(
                &Name::from_ascii("example.com.").unwrap(),
                RecordType::A,
                subnet,
                HOST,
            ),
            ResponsePlan::Forward
        );
    }

    #[test]
    fn short_name_resolves_within_the_uniquely_identified_callers_project() {
        let machine = MachineId::parse("a".repeat(32)).unwrap();
        let staging_web = in_project(
            observation(
                1,
                &machine,
                &ServiceId::parse("b".repeat(32)).unwrap(),
                &ServiceName::parse("web").unwrap(),
                ContainerKind::ServiceContainer,
                running(HealthObservation::Unhealthy),
                Some([10, 210, 1, 2]),
            ),
            "shop-staging",
        );
        let staging_database = in_project(
            observation(
                2,
                &machine,
                &ServiceId::parse("c".repeat(32)).unwrap(),
                &ServiceName::parse("database").unwrap(),
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 10]),
            ),
            "shop-staging",
        );
        let prod_web = in_project(
            observation(
                3,
                &machine,
                &ServiceId::parse("d".repeat(32)).unwrap(),
                &ServiceName::parse("web").unwrap(),
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 3]),
            ),
            "shop-prod",
        );
        let prod_database = in_project(
            observation(
                4,
                &machine,
                &ServiceId::parse("e".repeat(32)).unwrap(),
                &ServiceName::parse("database").unwrap(),
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 11]),
            ),
            "shop-prod",
        );
        let staging_cache = in_project(
            observation(
                5,
                &machine,
                &ServiceId::parse("f".repeat(32)).unwrap(),
                &ServiceName::parse("cache").unwrap(),
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 12]),
            ),
            "shop-staging",
        );
        let projection = Projection::from_observations(&[
            staging_web,
            staging_database,
            prod_web,
            prod_database,
            staging_cache,
        ]);
        let subnet = "10.210.1.0/24".parse().unwrap();
        let staging_caller = IpAddr::V4(Ipv4Addr::new(10, 210, 1, 2));
        let prod_caller = IpAddr::V4(Ipv4Addr::new(10, 210, 1, 3));

        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii("database.internal.").unwrap(),
                RecordType::A,
                subnet,
                staging_caller,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 10)]
        );
        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii("database.internal.").unwrap(),
                RecordType::A,
                subnet,
                prod_caller,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 11)]
        );
        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii("database.shop-prod.internal.").unwrap(),
                RecordType::A,
                subnet,
                staging_caller,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 11)]
        );
        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii("database.shop-staging.internal.").unwrap(),
                RecordType::A,
                subnet,
                HOST,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 10)]
        );
        assert!(
            addresses(projection.plan(
                &Name::from_ascii("database.internal.").unwrap(),
                RecordType::A,
                subnet,
                HOST,
            ))
            .is_empty()
        );
        assert!(
            addresses(projection.plan(
                &Name::from_ascii("cache.internal.").unwrap(),
                RecordType::A,
                subnet,
                HOST,
            ))
            .is_empty()
        );
        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii("cache.internal.").unwrap(),
                RecordType::A,
                subnet,
                staging_caller,
            )),
            vec![Ipv4Addr::new(10, 210, 1, 12)]
        );
    }

    #[test]
    fn hook_or_ambiguous_caller_gets_no_project_relative_resolution() {
        let machine = MachineId::parse("a".repeat(32)).unwrap();
        let web = ServiceName::parse("web").unwrap();
        let database = ServiceName::parse("database").unwrap();
        let staging_web = in_project(
            observation(
                1,
                &machine,
                &ServiceId::parse("b".repeat(32)).unwrap(),
                &web,
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 2]),
            ),
            "shop-staging",
        );
        let staging_database = in_project(
            observation(
                2,
                &machine,
                &ServiceId::parse("c".repeat(32)).unwrap(),
                &database,
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 10]),
            ),
            "shop-staging",
        );
        let hook = in_project(
            observation(
                3,
                &machine,
                &ServiceId::parse("b".repeat(32)).unwrap(),
                &web,
                ContainerKind::PreDeployHook,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 4]),
            ),
            "shop-staging",
        );
        let colliding = in_project(
            observation(
                4,
                &machine,
                &ServiceId::parse("d".repeat(32)).unwrap(),
                &web,
                ContainerKind::ServiceContainer,
                running(HealthObservation::Healthy),
                Some([10, 210, 1, 2]),
            ),
            "shop-prod",
        );
        let projection =
            Projection::from_observations(&[staging_web, staging_database, hook, colliding]);
        let subnet = "10.210.1.0/24".parse().unwrap();

        assert!(
            addresses(projection.plan(
                &Name::from_ascii("database.internal.").unwrap(),
                RecordType::A,
                subnet,
                IpAddr::V4(Ipv4Addr::new(10, 210, 1, 2)),
            ))
            .is_empty()
        );
        assert!(
            addresses(projection.plan(
                &Name::from_ascii("database.internal.").unwrap(),
                RecordType::A,
                subnet,
                IpAddr::V4(Ipv4Addr::new(10, 210, 1, 4)),
            ))
            .is_empty()
        );
        assert_eq!(
            addresses(projection.plan(
                &Name::from_ascii("database.shop-staging.internal.").unwrap(),
                RecordType::A,
                subnet,
                IpAddr::V4(Ipv4Addr::new(10, 210, 1, 4)),
            )),
            vec![Ipv4Addr::new(10, 210, 1, 10)]
        );
    }

    #[test]
    fn explicit_upstreams_override_system_configuration() {
        let upstreams = vec!["192.0.2.53:5353".parse().unwrap()];
        assert_eq!(
            configured_upstreams(Some(upstreams.clone()), Ipv4Addr::new(10, 210, 1, 1)),
            upstreams
        );
    }

    #[test]
    fn parses_resolv_conf_nameservers_skips_listen_and_junk() {
        let text = "\
# generated
search internal example.test
nameserver 192.0.2.53
nameserver 10.210.1.1
nameserver 2001:db8::53
nameserver not-an-ip
options ndots:1
	nameserver 198.51.100.53 # trailing comment
";
        assert_eq!(
            nameservers_from_resolv_conf(text, Ipv4Addr::new(10, 210, 1, 1)),
            vec![
                "192.0.2.53:53".parse().unwrap(),
                "[2001:db8::53]:53".parse().unwrap(),
                "198.51.100.53:53".parse().unwrap(),
            ]
        );
    }

    fn running(health: HealthObservation) -> ContainerRuntimeObservation {
        ContainerRuntimeObservation::Running { health }
    }

    fn observation(
        suffix: u8,
        machine_id: &MachineId,
        service_id: &ServiceId,
        service_name: &ServiceName,
        kind: ContainerKind,
        runtime: ContainerRuntimeObservation,
        address: Option<[u8; 4]>,
    ) -> ContainerObservation {
        ContainerObservation {
            container_id: ContainerId::parse(format!("{suffix:x}").repeat(64)).unwrap(),
            display_name: format!("{service_name}-{suffix}"),
            created_at_unix_nanos: 0,
            machine_id: *machine_id,
            project_name: ProjectName::parse("app").unwrap(),
            service_id: *service_id,
            service_name: service_name.clone(),
            kind,
            runtime,
            effective_healthcheck: None,
            resolved_spec: fixture_spec(service_id, service_name),
            address: address.map(|octets| ContainerAddress(octets.into())),
            labels: BTreeMap::new(),
        }
    }

    fn in_project(mut observation: ContainerObservation, project: &str) -> ContainerObservation {
        observation.project_name = ProjectName::parse(project).unwrap();
        observation
    }

    fn fixture_spec(service_id: &ServiceId, service_name: &ServiceName) -> ResolvedServiceSpec {
        serde_json::from_value(json!({
            "service_id": service_id,
            "name": service_name,
            "mode": { "mode": "replicated", "replicas": 1 },
            "container": { "image": "example.test/image", "pull_policy": "missing" }
        }))
        .unwrap()
    }

    fn addresses(plan: ResponsePlan) -> Vec<Ipv4Addr> {
        let ResponsePlan::Internal { answers, .. } = plan else {
            panic!("expected internal answer")
        };
        answers
            .into_iter()
            .map(|record| {
                let Some(IpAddr::V4(address)) = record.data.ip_addr() else {
                    panic!("expected A record, got {:?}", record.data)
                };
                address
            })
            .collect()
    }

    fn address_set(plan: ResponsePlan) -> std::collections::BTreeSet<Ipv4Addr> {
        addresses(plan).into_iter().collect()
    }
}
