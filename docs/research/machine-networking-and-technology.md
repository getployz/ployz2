# Machine networking and technology commitments

## Research boundary

This report describes the frozen Uncloud baseline at commit
`b7e224a1eff98813b1d1a32034d977be24be994e`. It covers machine initialization,
joining, removal, SSH provisioning, WireGuard topology, address allocation,
container networking, NAT behavior, endpoint selection, and peer propagation. It
classifies the technologies that implement those behaviors. It does not select
Rust libraries or redesign the system.

The source of truth for current behavior is the code at the frozen commit. The
older design note and networking article explain intent, but the current code
has evolved in one important way. The original note places the machine IPv4
address on WireGuard. The current implementation places a deterministic IPv6
management address on WireGuard and uses the first IPv4 address of the local
Docker subnet as the machine address. This report follows the current code while
retaining the original intent of direct, untranslated container routing.
[S1][S2][S3]

## Decision in one paragraph

Uncloud treats a cluster as a small, flat network of equal Linux machines. The
CLI bootstraps an ordinary remote host over SSH, then every machine runs the same
daemon, owns its local identity and WireGuard key, stores a full copy of cluster
state, and can act as a cluster entry point. Each machine receives one unique
IPv4 `/24` Docker bridge subnet. Its containers keep their real bridge addresses
when communicating across a full-mesh WireGuard network. A separate,
deterministic IPv6 address carries machine API and gossip traffic. Membership
rows from the eventually consistent store are projected into WireGuard peers.
There is no membership consensus, join transaction, fencing, relay, general
firewall abstraction, overlay controller, or network-level scheduler. These are
accepted limits, not missing layers that the Rust version should invent.
[S1][S4][S5][S6]

## The domain model the Rust version should preserve

The current Go structs mix configuration, persisted state, and runtime
observations. Rust can model these states more precisely without strengthening
the distributed semantics.

### Machine lifecycle

A machine has three meaningful persistent lifecycle states:

1. **Installed but uninitialized.** The daemon is running. It already owns a
   WireGuard key pair and a local Corrosion API token, but has no cluster machine
   ID, name, subnet, or management address. The empty machine ID is the current
   test for this state. [S7][S8]
2. **Joining.** The existing cluster has already created a membership row and
   assigned the target's ID, name, subnet, and management address. The target has
   persisted that assignment plus a snapshot of other machines and a minimum
   store version. It starts networking, but store-dependent components remain
   unavailable until the local replica reaches that version and fills known
   gaps. This is a real state in the behavior even though Go does not represent
   it as one enum. [S9][S10][S11]
3. **Initialized member.** The local machine state is authoritative for its own
   machine ID, name, key pair, advertised endpoints, public ingress IP, assigned
   subnet, management address, and WireGuard settings. The daemon periodically
   republishes this local information to the shared store. [S12][S13]

Reset is an asynchronous transition back to **installed but uninitialized**.
The RPC only marks the daemon as resetting and initiates shutdown because it
cannot remove the network and API it is using while still serving the RPC. On
shutdown, cleanup removes Uncloud-managed containers, the Docker network,
WireGuard link, firewall rules, Corrosion service data, and the machine state
directory. systemd then restarts the empty daemon. [S14][S15]

The stronger Rust model should therefore distinguish at least `Uninitialized`,
`Joining`, `Member`, and `Resetting` locally. It must not turn those local types
into a global membership protocol or a reconciliation engine.

### Identity and addressing

Each installed daemon generates its WireGuard key pair before it joins a
cluster. Cluster registration generates a separate opaque machine ID. Machine
names must be unique according to the registering member's current local view.
The default name is a normalized hostname with a numeric suffix if needed.
[S7][S16][S17]

The frozen address plan is:

| Address | Meaning |
| --- | --- |
| `10.210.0.0/16` by default | Cluster IPv4 allocation pool |
| One `/24` from that pool | One machine's local Docker bridge and container subnet |
| First usable IPv4 in the `/24`, usually `10.210.X.1` | Machine address and Docker bridge gateway |
| Remaining Docker-assigned IPv4 addresses | Service containers on that machine |
| `fdcc:` plus the first 14 public-key bytes | Deterministic machine management IPv6 address |
| Management IPv6 `/128` plus machine IPv4 `/24` | WireGuard `AllowedIPs` for one peer |

The in-memory IPAM rebuilds allocated ranges from the machine rows visible to
the member handling `AddMachine`, then takes the first free `/24`. With the
default `/16`, the mathematical ceiling is 256 machine subnets. A custom network
must be broad enough to contain at least one `/24`, although the CLI does not
validate that condition before registration. [S2][S16][S18]

The deterministic management IPv6 is an intentional simplification. It removes
the need for another coordinated allocator while separating management traffic
from container IPv4 traffic. Rust may give these addresses different types such
as `ManagementAddr`, `MachineSubnet`, `MachineGateway`, and `AdvertisedEndpoint`.
It should not add an address authority service.

### Endpoint state

Uncloud distinguishes two endpoint concepts that should not be collapsed:

- **Advertised endpoints** belong to a machine. They are local addresses and an
  externally observed public address, or an explicit CLI override. The owning
  machine persists and republishes them. [S19][S20]
- **Selected endpoint** belongs to one observer's relationship with one peer.
  It starts with the first advertised candidate, may change when the WireGuard
  kernel learns a reverse connection, and may rotate through candidates after a
  peer is judged down. It is persisted only in that observer's local peer state.
  [S21][S22]

This local distinction is important. There is no globally agreed "current
endpoint" for a machine.

## Lifecycle behavior

### Initialize the first machine

`uc machine init` currently requires a remote SSH destination. Local machine
initialization is explicitly a TODO. The command optionally installs the daemon,
checks whether the target is already initialized, asks before resetting an
existing membership, checks prerequisites, and sends the requested cluster
network, machine name, WireGuard port, MTU, endpoints, and public-IP policy to
the target. After success, it creates a local CLI context and saves the SSH
connection. [S23][S24]

The daemon then performs these operations:

1. Write the cluster network and creation time to the local replicated store.
2. Discover or accept explicit WireGuard endpoints and an optional ingress
   public IP.
3. Register itself through the same add-machine path used for later members.
4. Generate its machine ID, derive its management IPv6 from its existing
   WireGuard public key, allocate the first free `/24`, and create the membership
   row.
5. Persist the resulting member configuration in local machine state and signal
   the cluster controller to start.

[S25][S16][S26]

The controller configures the host firewall and local Docker bridge first, then
WireGuard, then the replicated store on the management address. It starts the
network API, waits for any required initial store synchronization, and only then
starts store-dependent services and announces cluster readiness. [S27]

### SSH bootstrap and host assumptions

The default remote path uses the host's `ssh` command so normal SSH config works.
A built-in Go SSH implementation is an explicit alternate scheme. The CLI
embeds the install script, base64-encodes it, streams it through the SSH shell,
and runs it as root or through passwordless sudo. A non-root SSH user is added to
the `uncloud` group so later SSH sessions can access the daemon's Unix socket.
[S28][S29]

The bundled installer supports Linux on amd64 or arm64 and requires systemd for
a normal install. It installs Docker when absent, creates a system user and
group, installs the daemon and uninstall script, writes a systemd unit, and
starts the service. `--no-install` bypasses this convenience path but assumes a
compatible daemon and its dependencies already exist. [S30][S31]

The architecture requires privileged Linux networking and a long-running daemon.
The exact shell encoding, package-manager probes, system SSH versus built-in SSH,
download URLs, and systemd unit text are implementation choices. Do not build a
provisioning framework to abstract every OS. Preserving the current narrow host
support is the smaller and more faithful choice.

### Join another machine

Join is a CLI-orchestrated sequence across an existing member and the target. It
is not a distributed transaction:

1. Connect to any existing member through the current context.
2. Provision or connect to the target over SSH.
3. If the target belongs to another cluster, ask to reset it first.
4. Ask the target for a token containing its existing WireGuard public key,
   public IP observation, and endpoint candidates.
5. Ask the existing member to register the target. That member chooses a unique
   name based on its current view, allocates the first visible free `/24`, derives
   the management IPv6, and writes the membership row.
6. Snapshot the entry member's per-actor replicated-store version and current
   machine list.
7. Send the assigned identity, other machines, minimum store version, port, and
   MTU directly to the target's `JoinCluster` RPC.
8. Save the target's SSH connection in the local CLI context.

[S32][S9][S16]

The target validates that the assigned public key matches the key it already
owns. It persists its assigned identity and a full initial peer list, then starts
the cluster controller. WireGuard and the network API start before store catch-up
so the replicated store can connect. Store-dependent services wait until the
captured version is reached and known gaps are filled. [S10][S11][S27]

Existing machines learn about the new member through their local machine-table
subscription. On every machine change, each member reloads the entire visible
machine list, reconstructs every peer, preserves a still-valid selected
endpoint, saves the peer list locally, and reapplies WireGuard. This is an
intentionally simple full projection, not an incremental membership protocol.
[S33]

### Remove a machine

Removal normally connects through a different member. The CLI refuses to remove
its current entry point while other members exist. By default it tries to reach
the target, lists and removes its service containers, initiates an asynchronous
reset, then deletes its container rows and membership row from the shared store.
It finally removes the matching SSH connection from the local context. If the
target is unreachable, or `--no-reset` is set, it deletes the shared rows without
cleaning the target. [S34][S35]

Other machines observe the deleted row and rebuild their peer sets without that
machine. There is no key-revocation list, fencing token, drain state, or atomic
guarantee that target reset completed before deletion. The source calls the
ordering optimistic and has a TODO for an unschedulable/removing state. A live
target left running with `--no-reset` still possesses its keys and local state.
The system does not promise that such a target is immediately or permanently
neutralized. [S34][S33]

This weakness must remain visible in the Rust code. A precise local enum for a
removal attempt is useful. A global drain controller, lease, consensus removal,
or fencing protocol is outside the preserved architecture.

## Data plane and management plane

### Full-mesh WireGuard

Each machine configures one WireGuard peer for every other visible machine. A
peer is allowed to route that machine's management IPv6 `/128` and its complete
IPv4 `/24`. The implementation sets a 25-second persistent keepalive on every
peer and adds kernel routes for the peer management address and container subnet.
The original networking article explicitly describes the topology as `N *
(N - 1)` peer configurations. [S4][S36][S37]

This gives direct machine-to-machine, machine-to-container, and
container-to-container paths without a central router. It also gives every
member a peer object, route, and shared-state relationship for every other
member. Full mesh is intended for relatively small clusters. Do not add route
reflectors, regional hubs, hierarchical membership, or partial peer graphs until
the product deliberately changes its operating envelope.

### Docker bridge and untranslated container addresses

Every machine owns a local-scope Docker bridge named `uncloud` with its assigned
subnet. Docker allocates container addresses from that subnet. The bridge MTU is
kept equal to the WireGuard MTU. Uncloud allows direct routing from the WireGuard
interface to the bridge, inserts a `DOCKER-USER` accept rule, and inserts a
`POSTROUTING` return rule ahead of Docker's masquerade rule. The last rule keeps
cross-machine container traffic from being source-NATed. Remote containers see
the actual source container address. [S38][S39]

The essential decision is one non-overlapping local container subnet per machine
plus direct routed packets that retain container identity. Docker bridge details
and the current iptables calls are how the frozen version realizes that decision.
Do not replace this with per-container proxies, exposed host ports, an ingress
path for east-west traffic, or a custom overlay protocol.

### Open trust inside the mesh

WireGuard authenticates peers, but the internal network is broad. A peer's whole
`/24` is allowed and the firewall accepts traffic from WireGuard to the Uncloud
bridge. There are no container ACLs or security groups. The original design
calls those possible future work, not part of the current network. There is also
a TODO to restrict the embedded registry to machine gateway addresses because
the current firewall rule permits traffic from the broader mesh. [S1][S36][S40]

The Rust version should preserve this trusted-cluster assumption and the TODO.
It should not invent an authorization policy language, identity-aware proxy, or
network-policy controller.

## NAT traversal and endpoint behavior

The CLI auto-discovers endpoint candidates from active, running, non-loopback
interfaces while excluding Docker, Uncloud, and Tailscale interfaces. It also
queries simple public-IP services and appends the result when available. Users
can override the candidates and listen port explicitly. [S19][S20]

For each peer, the local control loop:

1. Starts with the first advertised endpoint unless a previously selected
   endpoint remains advertised.
2. Polls the WireGuard device once per second.
3. Accepts WireGuard's learned endpoint when an inbound or reverse connection
   reveals a different source address.
4. Treats a newly selected endpoint as `Unknown` for 15 seconds.
5. Treats it as `Down` if no post-change handshake arrives after that window.
6. For an established peer, treats a handshake older than 275 seconds as down.
7. Rotates a down peer to the next advertised candidate.
8. Persists the selected endpoint in local machine state on a best-effort basis.

[S21][S22][S41]

This is deliberately modest NAT handling. Persistent keepalives maintain an
existing firewall or NAT mapping, WireGuard roaming learns a reverse endpoint,
and candidate rotation tries known addresses. There is no STUN coordination,
UDP hole punching service, TURN relay, DERP-like relay, or centralized endpoint
authority. The first-party article states the resulting limit plainly: at least
one side of each pair must be reachable, and two unrelated machines behind NAT
cannot connect without a relay. [S42]

The Rust version must not describe this as universal NAT traversal. It should
preserve endpoint states and failure visibility without adding a connectivity
control plane.

## Accepted failure semantics and implicit weaknesses

These behaviors fall out of the chosen design and should be recorded as design
ceilings, not automatically fixed:

- **Concurrent joins can collide.** The member handling a join reconstructs IPAM
  from its local machine rows and then writes the next `/24`. During a partition,
  two sides can see the same free subnet or name. The source includes a TODO to
  announce joins and achieve consensus, but the locked reconstruction direction
  explicitly preserves quorum-free operation. Carry the TODO as historical
  context. Do not implement consensus or refuse minority-partition operation.
  [S16][S43]
- **Join can partially succeed.** The membership row is created before the
  target receives `JoinCluster`. A later SSH or RPC failure can leave a ghost
  member. There is no rollback transaction. Operators can remove or retry it.
  [S32]
- **Initial sync can wait forever.** A joining node starts its network and waits
  without a failure deadline for the captured store version. It logs a warning
  every five minutes. Broken NAT or a stale peer list can leave it indefinitely
  not ready. [S11]
- **Membership updates are eventually projected.** Each node reacts to its
  local subscription. Peers can temporarily disagree about membership, routes,
  and selected endpoints. [S33]
- **Removal is not fencing.** Reset is asynchronous. An unreachable target is
  removed only from shared state. `--no-reset` intentionally leaves containers
  and data. No credential revocation proves the machine stopped participating.
  [S34][S35]
- **Scale is intentionally bounded.** Default address allocation provides 256
  `/24` ranges, full mesh grows quadratically, every change rebuilds all peers,
  and Corrosion currently bootstraps from every machine peer. The source has a
  TODO to use only a partial bootstrap list for large clusters. [S18][S4][S44]
- **Firewall support is narrow.** Current code programs iptables and ip6tables
  around Docker's chains. A TODO asks whether firewalld works. Do not preemptively
  build nftables, firewalld, or cross-platform backends. [S39]
- **Endpoint discovery is heuristic.** It trusts local interface enumeration and
  a few public-IP HTTP services. It skips Tailscale to avoid double tunneling.
  It does not validate candidate reachability before advertising them. [S19]
- **Custom network validation is late and incomplete.** The CLI parses a prefix,
  while `/24` allocation fails only during machine registration if the pool is
  too narrow. The documented contract calls it an IPv4 network. [S18][S45]
- **Internal isolation is intentionally absent.** All peer container subnets are
  routed and accepted. ACLs remain future work. [S1][S36][S39]

## Technology classification

The classifications below preserve the current design while separating it from
Go-shaped implementation details.

| Technology or choice | Classification | What must survive | What must not be inferred |
| --- | --- | --- | --- |
| Docker Engine as workload runtime | **Architectural commitment** | Existing Docker and Compose product model, local daemon ownership, and Docker-managed container lifecycle | Do not build a new runtime or a generic CRI layer |
| One local Docker bridge subnet per machine | **Architectural commitment** | Non-overlapping per-machine container addresses and direct cross-machine routing | The exact Docker SDK calls and bridge-name derivation are not domain concepts |
| WireGuard | **Architectural commitment** | Encrypted peer-to-peer mesh, machine-owned keys, kernel routing, keepalives, and endpoint roaming | Do not introduce a second overlay or a userspace packet protocol |
| Full mesh | **Architectural commitment and accepted ceiling** | Every visible machine directly peers with every other machine | Do not add hubs, relays, route reflectors, or topology optimization speculatively |
| IPv4 `/24` data plane plus deterministic IPv6 management plane | **Architectural commitment** | Separate container and management address roles, no management IP allocator, stable routable identities | Rust can use stronger types, but should not add an IPAM service |
| Direct routing without NAT inside the mesh | **Architectural commitment** | Preserve source container IPs and avoid per-container proxies or exposed host ports | Internet egress may still use Docker NAT |
| SSH bootstrap from `uc machine init/add` | **Product and architectural commitment** | Provision an ordinary remote host and reach its local daemon without a control plane | The shell transport, embedded script encoding, and SSH implementation are incidental |
| Linux privileged networking | **Architectural platform constraint** | Kernel WireGuard, routes, Docker bridge, and firewall mutation on supported Linux hosts | Do not spend owned code on unsupported host operating systems |
| systemd service management | **Replaceable mechanism with current narrow support** | A supervised daemon restart is needed for reset and boot persistence | Do not create a service-manager abstraction before another supported manager exists |
| iptables and ip6tables | **Replaceable mechanism, current supported implementation** | Permit mesh and DNS traffic, preserve container IPs, and constrain exposed daemon ports | Do not generalize to every firewall backend while current support is deliberately narrow |
| Corrosion for replicated machine rows and subscriptions | **Replaceable mechanism** | Local availability, eventual propagation, subscriptions, and a joining member's minimum-version catch-up | Do not replace it with consensus or stronger global truth as a side effect |
| Corrosion gossip over management IPv6 | **Replaceable mechanism** | Replication must travel inside the mesh and work without a central coordinator | QUIC details, container packaging, ports, and bootstrap-list construction are incidental |
| gRPC machine API and transparent proxy | **Replaceable mechanism** | The CLI can reach one member, target another directly, and aggregate or proxy cluster operations | There is no interoperability requirement, so protobuf layout is not sacred |
| `wgctrl`, netlink, Docker Go client, Cobra, protobuf | **Incidental implementation choices** | Their observable behavior only | Do not mirror Go package boundaries or APIs in Rust |
| Public-IP HTTP services | **Incidental implementation choice** | Best-effort public address discovery plus explicit override | These services are not a reliable discovery or coordination plane |
| Talos KubeSpan-inspired status timers and endpoint rotation | **Replaceable mechanism** | Distinguish unknown, up, and down and try known candidates without coordination | Exact timer constants and borrowed Go structure need not dictate Rust types |
| Embedded bash installer and `get.docker.com` | **Incidental convenience** | Default install remains easy and uses the least owned provisioning code | Do not build a package repository or configuration-management system |

“Replaceable” does not mean “replace now.” Under the least-owned-code rule, the
existing tool remains the default unless a reconstruction constraint proves it
cannot serve. It only means the architecture depends on its properties rather
than its brand or source-level API.

## TODOs that carry architectural information

The broader TODO inventory belongs in its own research ticket. The following
TODOs are directly relevant to machine networking and must be preserved as
comments or explicitly dispositioned during reconstruction:

| Frozen TODO | Meaning for reconstruction |
| --- | --- |
| Support local `machine init` [S23] | Preserve the missing behavior. Do not silently add it during remote bootstrap work. |
| Announce a new member and achieve consensus [S43] | Preserve as historical pressure. The locked design rejects consensus and minority fencing. Do not implement it. |
| Automatically avoid connecting through the machine being removed [S34] | Preserve the current CLI limitation. |
| Mark a removing machine unschedulable [S34] | Preserve the race and comment. Do not grow a drain controller incidentally. |
| Warn that DNS records may need updating after Caddy machine removal [S34] | Preserve as an operator-facing follow-up, not a networking transaction. |
| Check link-layer interface type during endpoint discovery [S19] | Preserve the heuristic ceiling. |
| Verify behavior under firewalld [S39] | Preserve unsupported uncertainty. Do not add a multi-firewall abstraction. |
| Restrict embedded-registry access to machine gateway IPs [S40] | Preserve the known broad internal access. |
| Use only part of the peer list for Corrosion bootstrap at large scale [S44] | Preserve the small-cluster ceiling. Do not optimize topology in advance. |
| Tighten Corrosion config permissions [S44] | Preserve the local ownership and permissions debt. |
| Install the CLI and create a `uc` alias on machines [S46] | Preserve as installation convenience, not a networking prerequisite. |

Version-compatibility and migration-removal TODOs also appear in these files.
They should be included in the global TODO ledger, but they do not define the
new networking architecture.

## Reconstruction guardrails

Use these rules when an implementation agent is tempted to improve this area:

1. Model local lifecycle and address roles with strong Rust enums and newtypes.
   Do not use those types to promise stronger global state.
2. Keep WireGuard, Docker, the flat full mesh, unique `/24` bridge subnets,
   deterministic management IPv6 addresses, and untranslated container routing.
3. Keep joins and removals imperative and best-effort. Report partial failure.
   Do not add a transaction coordinator, leases, fencing, or continuous desired
   state.
4. Keep endpoint handling local. Try advertised candidates, accept WireGuard
   roaming, and expose unknown, up, and down. Do not add relay infrastructure.
5. Keep the supported host envelope narrow. Reuse the platform and Docker
   features already doing the work. Do not create portability layers for
   hypothetical hosts or firewall stacks.
6. Reconfigure the whole peer set on a machine-table change until measurements
   prove this simple approach fails in the supported cluster size.
7. Place short rationale comments next to tempting changes. For example:
   `ponytail: full mesh and whole-set rebuild are deliberate for small clusters.
   Reconsider only when measured cluster size makes them fail.`
8. Preserve the TODO comments listed above. A TODO records a known ceiling. It
   is not permission for an agent to implement the missing machinery.

## Sources

[S1]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/misc/design.md#L20-L47
[S2]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/network/ip.go#L11-L21
[S3]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/network/wireguard_linux.go#L72-L116
[S4]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/blog/2025-08-01-wireguard-overlay/index.md#L392-L398
[S5]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/misc/design.md#L49-L74
[S6]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster.go#L632-L744
[S7]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L200-L245
[S8]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L331-L338
[S9]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/cli.go#L455-L494
[S10]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L878-L980
[S11]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster.go#L369-L445
[S12]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/state.go#L17-L39
[S13]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster.go#L485-L563
[S14]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L1305-L1327
[S15]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster.go#L746-L762
[S16]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/cluster.go#L85-L183
[S17]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/machine.go#L11-L65
[S18]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/ipam.go#L11-L81
[S19]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/network/address.go#L15-L88
[S20]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/machine/add.go#L78-L137
[S21]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/network/peer.go#L27-L72
[S22]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster.go#L681-L743
[S23]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/cli.go#L183-L290
[S24]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/machine/init.go#L44-L171
[S25]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/cluster.go#L50-L63
[S26]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L761-L875
[S27]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster.go#L121-L270
[S28]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/machine.go#L21-L101
[S29]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/cli.go#L530-L612
[S30]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/scripts/install.sh#L53-L139
[S31]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/scripts/install.sh#L142-L159
[S32]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/cli.go#L330-L520
[S33]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster.go#L632-L744
[S34]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/machine/rm.go#L58-L188
[S35]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/cluster.go#L244-L269
[S36]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/network/config.go#L63-L120
[S37]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/network/wireguard_linux.go#L209-L279
[S38]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/controller_linux.go#L26-L121
[S39]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/controller_linux.go#L124-L168
[S40]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/firewall/iptables_linux.go#L21-L73
[S41]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/network/peer.go#L75-L173
[S42]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/blog/2025-08-01-wireguard-overlay/index.md#L400-L422
[S43]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/cluster.go#L149-L177
[S44]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L633-L689
[S45]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_machine_init.md#L34-L58
[S46]: https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/scripts/install.sh#L276-L276
