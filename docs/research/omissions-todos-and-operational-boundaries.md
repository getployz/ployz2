# Uncloud omissions, TODOs, and operational boundaries

## Scope and baseline

This report inventories omissions in Uncloud at commit
[`b7e224a1eff98813b1d1a32034d977be24be994e`](https://github.com/psviderski/uncloud/tree/b7e224a1eff98813b1d1a32034d977be24be994e).
It focuses on behavior that a Rust reconstruction might accidentally strengthen. It also accounts for every authored
`TODO`, `FIXME`, `XXX`, `HACK`, `NOT IMPLEMENTED`, and `Not supported yet` marker in the frozen tree.

The scan found 151 explicit authored markers after excluding dependencies, generated code, lockfiles, and two false
positives where `xxx` and `br-XXX` were only parser and interface-name placeholders. All 151 are listed in the inventory
below. There are no authored `FIXME`, `XXX`, or standalone `HACK` markers. The two comments that use the word `hack` are
themselves `TODO` markers and are included.
Generated gRPC `Unimplemented*Server` boilerplate is not an omission and is not counted.

## Reconstruction rule

The Rust project should preserve a TODO beside the equivalent boundary by default. A TODO does not grant permission to
add coordination, reconciliation, storage replication, stronger consistency, or a new abstraction. If Rust or a selected
dependency removes the construct that a Go-specific TODO referred to, keep the marker in an upstream TODO ledger and mark
it `not applicable` with the reason. Never create dead Rust code only to host an old comment.

Use these dispositions in the inventory:

- **Preserve boundary** means the absent behavior or weakness affects observable semantics. Carry the comment into Rust
  beside the equivalent decision.
- **Carry TODO** means the gap is product relevant but does not define the architecture. Carry it until a later parity
  decision resolves it.
- **Resolve by structure** means the marker concerns Go code shape or a selected library. Record its disposition, but do
  not reproduce the Go problem.
- **Migration cleanup** means compatibility with old Uncloud releases. The Rust version has no interoperability goal, so
  record and close it as not applicable.
- **Reference only** means the marker lives in an experiment, website UI, test maintenance, or build tooling. It still
  belongs in the ledger, but it does not create Rust runtime work by itself.

## Architectural ceilings to preserve

### Availability wins over a single authoritative truth

Every machine may accept commands and continue operating in a partition. Shared state converges without coordination.
Convergence can produce a result that neither user intended. This is an accepted consequence, not an invitation to add
quorum writes, a leader, fencing, or a consensus-backed control plane. The original design states the AP choice and the
semantic cost directly ([design lines 49-74](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/misc/design.md#L49-L74)).

The current code still adds a machine by writing to the local replicated store. Its TODO explicitly says there is no
announcement or consensus and that a minority partition may proceed
([cluster/cluster.go lines 163-178](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/cluster.go#L163-L178)).
Service inspection also acknowledges two direct consequences. Container status may not be trusted, and concurrent
creation can produce multiple service IDs with the same name
([machine.go lines 1330-1368](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L1330-L1368)).

Rust may represent `trusted`, `stale`, `conflicted`, and `unknown` states with better enums. It must not manufacture a
stronger distributed guarantee behind those types.

### Imperative scheduling, not an autonomous reconciler

Uncloud deliberately favors direct commands because their errors are more predictable. Shared-state reactions are used
for projections such as DNS and ingress, not as a general desired-state engine
([design lines 99-122](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/misc/design.md#L99-L122)).
Docker restarts a failed container on its current machine. Uncloud does not move it to another machine after a failure.
Global services also do not appear on a newly added machine until the user runs `uc deploy` again
([global service guide lines 23-32](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/4-guides/1-deployments/3-deploy-global-services.md#L23-L32)).

Deployment planning is a one-shot snapshot followed by an ordered operation sequence. The sequence stops at the first
error and does not undo prior successful operations
([deployment lines 292-331](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/deploy.go#L292-L331),
[sequence lines 9-20](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/operation/sequence.go#L9-L20)).
Do not add background convergence, automatic global scaling, automatic rescheduling, or transactional deployment.

### Partial failure and rollback are normal states

Rolling updates replace containers one at a time. A failed replacement stops the sequence. A successful earlier
replacement remains in place, later containers remain untouched, and the failed container is retained for inspection
([rolling guide lines 155-181](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/4-guides/1-deployments/4-rolling-deployments.md#L155-L181)).
For `stop-first`, rollback only attempts to restart the old container. That restart may itself fail. Errors from stopping
the failed new container are ignored
([container operation lines 173-237](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/operation/container.go#L173-L237)).

The default `start-first` update has an acknowledged race. The new container reaches Caddy through asynchronous store
propagation, so a single-replica service may have brief downtime when the old container stops before Caddy observes the
new one
([container operation lines 240-256](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/operation/container.go#L240-L256)).
Preserve this TODO. Do not add deployment transactions, distributed barriers, or a Caddy acknowledgement protocol unless
a later decision explicitly changes the architecture.

### Machine removal is optimistic and does not relocate work

A machine is not marked unschedulable while removal runs. Reset is asynchronous and optimistic. An unreachable machine
is simply removed without reset. Cluster records are deleted, but service replicas and data are not recreated elsewhere
([machine removal lines 98-166](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/machine/rm.go#L98-L166),
[cluster removal lines 244-269](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/cluster.go#L244-L269)).
Removal also does not update the managed public DNS records automatically
([machine removal lines 169-188](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/machine/rm.go#L169-L188)).

## Operational boundary matrix

| Area | Preserved behavior and weakness | What Rust must not infer |
|---|---|---|
| DNS projection | Internal DNS returns healthy service containers from the replicated store. It does not filter records by the hosting machine's Corrosion membership state yet ([resolver lines 38-105](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/dns/resolver.go#L38-L105)). | Do not turn DNS into a failure detector or authoritative scheduler view. A healthy record on an unavailable machine can remain visible. |
| DNS protocol | The internal authority handles only IPv4 `A` queries. It uses TTL 0, returns NXDOMAIN when no record exists, and forwards non-internal queries to system resolvers ([server lines 186-239](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/dns/server.go#L186-L239), [server lines 293-344](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/dns/server.go#L293-L344)). | Do not add SRV, TXT, AAAA, caching, virtual service IPs, or a service load balancer as hidden parity work. |
| Managed public DNS | The hosted Uncloud DNS service is used directly. Its bearer token is stored unencrypted in replicated state. `release` deletes only the local record because the service-side release call is not implemented ([cluster DNS lines 16-63](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/dns.go#L16-L63), [lines 97-112](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/dns.go#L97-L112)). | Do not rebuild the hosted DNS service in this effort. Do not silently add a secret store or lifecycle manager. Preserve the plaintext and release TODOs. |
| Ingress | Caddy runs as a global service on every machine by default and handles HTTP, HTTPS, certificates, and load balancing ([ingress overview lines 5-25](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/3-concepts/2-ingress/1-overview.md#L5-L25)). TCP and UDP use host mode. L4 ingress through Caddy is not implemented ([publishing lines 30-64](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/3-concepts/2-ingress/2-publishing-services.md#L30-L64), [Caddy generator lines 290-330](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/caddyconfig/caddyfile.go#L290-L330)). | Do not introduce an L4 proxy, distributed VIP, BGP, anycast, or a new ingress controller. |
| Ingress health | Each daemon rebuilds local Caddy config from healthy replicated container records. Like DNS, it does not yet filter by machine membership ([controller lines 92-146](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/caddyconfig/controller.go#L92-L146)). | Do not promise that membership loss immediately removes a remote upstream. |
| Caddy validation | User Caddy snippets are adapted before load, but adaptation cannot prove the config will load. A load failure keeps the last successful config and retries on a later container change ([Caddy client lines 91-136](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/caddyconfig/client.go#L91-L136), [controller lines 149-189](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/caddyconfig/controller.go#L149-L189)). | Do not build a complete Caddy validator or transactional config distributor. |
| Secrets | Secrets resolve on the CLI machine at deploy time and become environment values. They are stored unencrypted in the distributed service spec and in Docker container config. File-mounted secrets are unsupported ([secrets lines 35-48](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/3-concepts/8-secrets.md#L35-L48), [lines 133-141](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/3-concepts/8-secrets.md#L133-L141)). | Do not add encryption at rest, secret rotation, a provider control plane, file injection, or cluster-side credential storage as parity work. |
| Configs | Config content is read by the CLI, sent inside the service spec, copied per container, and deleted with that container. Changes require redeployment. External configs and short syntax are unsupported ([configs lines 157-171](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/3-concepts/7-configs.md#L157-L171), [lines 206-210](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/3-concepts/7-configs.md#L206-L210)). | Do not invent a persistent config object store or live config projection. |
| Storage | Named volumes are Docker volumes on specific machines. Existing volume location constrains placement. A missing replicated-service volume is created on one eligible machine. A global-service volume is created independently on every eligible machine ([volume scheduler lines 12-20](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/scheduler/volume.go#L12-L20), [lines 187-249](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/scheduler/volume.go#L187-L249)). | Same volume name does not mean replicated data. Do not add migration, replication, backup, attachment fencing, or a CSI-like subsystem. |
| Storage update safety | A single-replica service with a named volume defaults to `stop-first`. Multi-replica services are assumed to allow concurrent access. Bind and tmpfs mounts do not trigger the safety switch ([rolling guide lines 22-41](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/4-guides/1-deployments/4-rolling-deployments.md#L22-L41)). | Do not infer storage semantics or coordinate leases. Preserve the simple heuristic and user override. |
| Health | A running, non-paused, non-restarting container without a Docker healthcheck counts as healthy. A healthchecked container counts as healthy only when Docker says so ([container lines 55-80](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/container.go#L55-L80)). | Do not add active application probes outside Docker healthchecks. |
| Post-deploy health | After deployment, an unhealthy container is removed from Caddy and added back if it recovers. Uncloud does not restart it or roll it back ([rolling guide lines 104-140](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/4-guides/1-deployments/4-rolling-deployments.md#L104-L140)). | Do not add a service controller that repairs unhealthy replicas. Docker restart policy remains responsible for local restart. |
| Trust and authorization | SSH and local access are authorized by the machine's `root` user or `uncloud` Unix group ([connecting lines 49-68](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/3-concepts/1-clusters/1-connecting.md#L49-L68)). On the mesh, the firewall allows the machine API from the management WireGuard range and the gRPC connector uses no additional transport credentials ([firewall lines 51-71](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/firewall/iptables_linux.go#L51-L71), [WireGuard connector lines 53-79](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/connector/wireguard.go#L53-L79)). | Treat cluster membership and machine access as the trust boundary. Do not add tenants, roles, per-service ACLs, mTLS identity, or policy machinery without a new product decision. |
| Compose scope | Uncloud intentionally accepts only a subset of Compose. Unsupported items include custom DNS, labels, links, custom networks, several memory and security options, Compose placement, restart policy, rollback config, external configs, and config short syntax ([support matrix lines 13-76](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/8-compose-file-reference/1-support-matrix.md#L13-L76)). | Do not chase full Compose compatibility. The parity classification wave must explicitly choose additions and exclusions. |
| Service lifecycle | `depends_on: service_completed_successfully` is rejected because services are long-running and independently owned. Uncloud points users to a pre-deploy hook instead ([compose validation lines 501-514](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/service.go#L501-L514)). | Do not introduce Jobs or cross-service lifecycle ownership as an implicit Compose feature. |
| Platform | The daemon and network management are Linux-only. Darwin implementations return explicit unsupported errors ([HACKING lines 40-46](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/HACKING.md#L40-L46), [WireGuard Darwin stub](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/network/wireguard_darwin.go#L13-L30)). | Rust does not need a cross-platform daemon abstraction. The CLI can remain cross-platform while the daemon targets Linux. |

## Complete explicit-marker inventory

Each row lists every marker line in that file. A range in the source link spans all listed marker lines. This table accounts
for all 151 explicit authored markers.

| Source and marker lines | Disposition | What the markers cover |
|---|---|---|
| [`pkg/api/resources.go` 18](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/resources.go#L13-L22) | Preserve boundary | Memory-aware placement is absent. Do not add a capacity scheduler as incidental parity work. |
| [`pkg/api/config.go` 20, 24, 28](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/config.go#L13-L29) | Carry TODO | External configs, config labels, and environment-sourced configs are not implemented. |
| [`pkg/api/volume.go` 54, 91](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/volume.go#L29-L93) | Preserve boundary | The need for driver and label fields is questioned under the externally managed volume model. Bind propagation defaults remain implicit. |
| [`pkg/api/service.go` 34, 39, 166, 294](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/service.go#L26-L295) | Mixed, carry all | Cluster-local image pulling is absent, port conflict validation is incomplete, and a deprecated volume field awaits removal. The image and port gaps are behavioral. The deprecated field is structural cleanup. |
| [`pkg/api/container.go` 210](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/container.go#L203-L230) | Carry TODO | Published ports still come from container labels, so ingress-port-only changes recreate containers. |
| [`pkg/api/machine.go` 40](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/machine.go#L31-L45) | Resolve by structure | The Go client should use domain types instead of protobuf types. Rust should solve this directly with strong domain types. |
| [`pkg/client/client.go` 23](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/client.go#L15-L29) | Resolve by structure | Go client embedding exposes more methods than required. |
| [`pkg/client/connector/ssh.go` 52](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/connector/ssh.go#L38-L60) | Carry TODO | SSH connection establishment does not fully honor context cancellation. |
| [`pkg/client/service.go` 128, 347, 358](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/service.go#L118-L365) | Preserve boundary plus cleanup | Broadcast failures are warned and omitted rather than returned as typed partial results. Service extraction makes extra calls. Preserve the partial-result semantics unless parity explicitly changes them. |
| [`pkg/client/connector/wireguard.go` 36, 41](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/connector/wireguard.go#L29-L49) | Carry TODO | Cancellation is incomplete and only the first configured machine is tried. |
| [`pkg/client/image.go` 40, 167, 179, 402, 529, 562](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/image.go#L34-L563) | Mixed, carry all | Third-party packaging risk, duplicate machine truth, platform mismatch, quiet push, rootless virtualized handling, and a hard-coded image remain open. Use the best Rust tools rather than copying Go packaging. |
| [`pkg/client/machine.go` 18, 132](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/machine.go#L13-L136) | Resolve by structure and migration cleanup | One client path should use another RPC. An old gRPC compatibility branch awaits removal. |
| [`pkg/client/volume.go` 74](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/volume.go#L51-L87) | Preserve boundary | Failed machines in a volume broadcast are warned and omitted from the returned value. |
| [`pkg/client/container.go` 24, 58, 183, 477](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/container.go#L20-L478) | Carry TODO | Progress formatting, service name consistency, quiet pull, and richer health-wait feedback remain open. |
| [`experiment/syncer.go` 18, 19, 47, 137, 154](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/experiment/syncer.go#L15-L158) | Reference only | Experimental Merkle-DAG session optimization, persistence, peer exclusion, broadcast efficiency, and module ownership. Do not port this experiment merely because it exists. |
| [`cmd/uc/caddy/deploy.go` 207](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/caddy/deploy.go#L199-L214) | Resolve by structure | Split record discovery from record mutation confirmation. |
| [`cmd/uc/service/exec.go` 111](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/service/exec.go#L102-L117) | Carry TODO | TTY behavior mirrors Compose instead of detecting the terminal dynamically. |
| [`experiment/broadcaster.go` 33](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/experiment/broadcaster.go#L25-L38) | Reference only | Experimental CRDT broadcasts could include content to avoid a round trip. |
| [`pkg/client/deploy/scheduler/state.go` 30](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/scheduler/state.go#L23-L39) | Resolve by structure | Cluster scheduling state takes multiple API broadcasts instead of one richer inspection. |
| [`experiment/serf_crdt.go` 29, 169](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/experiment/serf_crdt.go#L23-L172) | Reference only | Experimental logging integration and a backlog-related DAG-head growth bug. |
| [`cmd/uc/service/run.go` 102](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/service/run.go#L94-L107) | Preserve boundary | CLI `run` does not publish L4 TCP or UDP ingress. Host mode remains the supported path. |
| [`pkg/client/deploy/scheduler/volume.go` 28](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/scheduler/volume.go#L20-L33) | Resolve by structure | Volume-to-service tracking assumes every spec has a service name. Rust types can remove this invalid state. |
| [`cmd/uc/service/scale.go` 100](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/service/scale.go#L91-L106) | Preserve boundary | Scaling a service whose containers disagree on spec simply chooses one. User selection is not implemented. |
| [`website/docs/3-concepts/8-secrets.md` 47](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/3-concepts/8-secrets.md#L42-L50) | Preserve boundary | Secret file mounts are unsupported. |
| [`pkg/client/compose/predeploy_test.go` 46](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/predeploy_test.go#L35-L60) | Carry TODO | Unknown pre-deploy attributes are ignored rather than rejected. |
| [`cmd/uc/machine/init.go` 66](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/machine/init.go#L57-L70) | Preserve boundary | Local machine initialization is not supported. |
| [`cmd/uc/machine/add.go` 190](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/machine/add.go#L175-L206) | Preserve boundary | Adding a machine performs another Caddy deployment and may cause small downtime. It is not automatic global-service reconciliation. |
| [`pkg/client/deploy/scheduler/constraint.go` 26, 27](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/scheduler/constraint.go#L22-L33) | Preserve boundary | Scheduling does not consider image platform compatibility or local image presence under `pull_policy: never`. |
| [`pkg/client/compose/config.go` 12, 45](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/config.go#L11-L52) | Mixed | Config short syntax is absent. File-path handling could be factored. Carry the former and solve the latter naturally. |
| [`cmd/uc/machine/rm.go` 59, 98, 185](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/machine/rm.go#L58-L186) | Preserve boundary | Connection selection is manual, removal has no unschedulable state, and public DNS is not updated. |
| [`pkg/client/deploy/scheduler/service.go` 51](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/scheduler/service.go#L41-L54) | Carry TODO | An unused scheduling method has no heap-based implementation and returns `not implemented`. Do not invent a general scheduler if the active strategy does not need it. |
| [`website/src/theme/DocBreadcrumbs/index.js` 10, 25, 33](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/src/theme/DocBreadcrumbs/index.js#L1-L35) | Reference only | Website component placement and Google breadcrumb behavior. |
| [`pkg/client/deploy/container_test.go` 649, 848, 875, 903, 931, 1367](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/container_test.go#L640-L1372) | Carry TODO | Unused volumes, several mutable spec changes, and bind propagation equivalence currently force recreation or remain ambiguous. |
| [`pkg/client/deploy/container.go` 48, 52, 66, 69](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/container.go#L40-L72) | Carry TODO | Mutable fields and ingress ports recreate containers because in-place spec updates are absent. Unused volume definitions are unresolved. |
| [`pkg/client/deploy/operation/container.go` 45, 101, 243](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/operation/container.go#L39-L246) | Mixed, preserve all | Compose progress forces an event-ID hack, stop output lacks a service name, and asynchronous Caddy propagation can cause brief downtime. The last marker is an architectural failure boundary. |
| [`cmd/uc/deploy.go` 80](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/deploy.go#L73-L85) | Carry TODO | Deploy has no machine filter that preserves containers on excluded machines. |
| [`pkg/client/compose/service.go` 129, 462](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/service.go#L120-L130) | Carry TODO | Tmpfs translation and complete detection of commonly used unsupported Compose fields remain incomplete. See also [line 462](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/service.go#L455-L463). |
| [`pkg/client/deploy/strategy.go` 63, 77, 173, 334](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/strategy.go#L56-L78) | Preserve boundary plus TODO | `pull_policy: never` does not constrain by image presence, constraint errors lack a detailed report, and mutable-field updates are not planned. See [update markers](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/strategy.go#L165-L175) and [line 334](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/strategy.go#L326-L336). |
| [`pkg/client/logs.go` 71, 177](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/logs.go#L60-L180) | Carry TODO | Already-opened log streams can live until parent context cancellation when another stream open fails. |
| [`Makefile` 1](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/Makefile#L1) | Reference only | Upstream intends to replace Make targets with Mise tasks. |
| [`pkg/client/deploy/deploy.go` 79, 94, 373](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/deploy.go#L73-L95) | Carry TODO | Deployment-based removal and full spec diffs are absent. The same deployment object may be run more than once. See [line 373](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/deploy.go#L370-L380). |
| [`pkg/client/compose/deploy.go` 102, 141](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/deploy.go#L94-L145) | Preserve boundary plus TODO | `depends_on` conditions are not fully represented in operation planning. Scheduling uses unresolved rather than resolved specs. |
| [`.github/workflows/go-tests.yml` 72](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/.github/workflows/go-tests.yml#L66-L76) | Reference only | Go platform lockfile workaround. |
| [`test/e2e/service_test.go` 301, 376, 779, 1491, 1542, 1805](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/test/e2e/service_test.go#L296-L302) | Mixed, carry all | In-place placement updates, richer constraint failures, unreachable-machine deployment coverage, fixture field drift, and L4 TCP ingress remain open. See [lines 1491-1542](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/test/e2e/service_test.go#L1491-L1542) and [lines 1798-1816](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/test/e2e/service_test.go#L1798-L1816). |
| [`internal/machine/corromigrate/migrate.go` 3](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/corromigrate/migrate.go#L1-L8) | Migration cleanup | Pre-v1 Corrosion migration removal after Uncloud 0.22. No interoperability means not applicable. |
| [`internal/machine/machine.go` 561, 566, 652, 687, 1346, 1347, 1379](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L553-L568) | Mixed, carry all | Shutdown timeouts, large-cluster bootstrap fanout, file ownership, trusted status, conflicting service identities, and duplicated log code. See [bootstrap and permission lines](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L646-L690) and [service lines](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L1339-L1382). |
| [`internal/machine/firewall/iptables_linux.go` 37](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/firewall/iptables_linux.go#L30-L44) | Preserve boundary | The embedded registry is reachable from cluster containers, not only machine IPs. Preserve the trust TODO. |
| [`internal/machine/store/container.go` 214](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/store/container.go#L205-L220) | Resolve by structure | Stored `sync_status` may be unused. Rust should model or remove it explicitly. |
| [`internal/machine/docker/client_exec.go` 202](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/client_exec.go#L196-L208) | Resolve by structure | Client logic is split unnecessarily across packages. |
| [`internal/machine/network/address.go` 48](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/network/address.go#L30-L52) | Carry TODO | Routable-address discovery may include link-layer interfaces that should be filtered. |
| [`internal/machine/docker/client.go` 27](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/client.go#L20-L31) | Resolve by structure | The intermediate Docker client may add no value. |
| [`internal/machine/dns/server.go` 221, 337](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/dns/server.go#L211-L222) | Preserve boundary | Non-A record types and nonzero caching TTL are absent. See [TTL lines](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/dns/server.go#L329-L340). |
| [`internal/machine/caddyconfig/caddyfile.go` 323](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/caddyconfig/caddyfile.go#L310-L326) | Preserve boundary | L4 TCP and UDP ingress routing is not implemented. |
| [`internal/machine/docker/controller_linux.go` 110](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/controller_linux.go#L100-L116) | Carry TODO | Docker network firewall behavior with firewalld is unverified. A later `br-XXX` example is a lexical false positive, not a marker. |
| [`internal/machine/caddyconfig/controller.go` 101, 122, 133](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/caddyconfig/controller.go#L92-L135) | Mixed | Legacy JSON output awaits removal. Machine membership is not used to suppress likely unreachable upstreams. Preserve the latter failure boundary. |
| [`internal/machine/dns/resolver.go` 46, 63](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/dns/resolver.go#L38-L65) | Preserve boundary | DNS does not use machine membership to suppress likely unreachable containers. |
| [`internal/machine/caddyconfig/client.go` 132](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/caddyconfig/client.go#L128-L136) | Preserve boundary | Caddy adaptation is the only validation. Full validation through `docker exec` or a module is absent. |
| [`internal/machine/docker/controller.go` 159](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/controller.go#L151-L166) | Carry TODO | A network recreation does not mark all stored containers outdated. |
| [`internal/machine/cluster.go` 51, 567, 607](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster.go#L45-L54) | Mixed | Old Corrosion migration and schema cleanup are versioned migration work. Docker ownership is misplaced. See [later markers](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster.go#L560-L612). |
| [`internal/machine/docker/server.go` 75, 515, 589, 992, 999](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/server.go#L68-L82) | Mixed, carry all | Possibly redundant network readiness checks, DB ownership split, port-label migration, and uninvestigated non-local volume behavior. See [server split and labels](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/server.go#L510-L592) and [volume lines](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/server.go#L985-L1002). |
| [`internal/sshexec/ssh.go` 45, 87](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/sshexec/ssh.go#L35-L90) | Carry TODO | Built-in SSH tries only fixed key paths and cannot prompt for an encrypted private key. The system SSH path remains the broad-compatibility option. |
| [`internal/grpcversion/interceptor.go` 51, 129, 139, 148, 173, 179, 184](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/grpcversion/interceptor.go#L45-L185) | Migration cleanup | All seven markers retire transitional protocol-version response handling. No interoperability means not applicable. |
| [`internal/docker/client.go` 19](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/docker/client.go#L13-L24) | Resolve by structure | A helper should be a client method. |
| [`internal/machine/api/proxy/director.go` 108](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/api/proxy/director.go#L98-L112) | Carry TODO | Cached remote gRPC backends for removed machines are not periodically closed. |
| [`internal/machine/cluster/cluster.go` 174](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/cluster.go#L163-L178) | Preserve boundary | Machine addition has no announcement, consensus, minority check, or fence. |
| [`internal/machine/cluster/dns.go` 23, 110](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/dns.go#L16-L25) | Preserve boundary | Managed DNS token is plaintext in the store. Service-side domain release is absent. See [release lines](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/dns.go#L97-L112). |
| [`internal/cli/cli.go` 189, 228, 389, 464, 496](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/cli.go#L183-L232) | Mixed | Local initialization is absent. Three old RPC compatibility fallbacks await removal. Current-context errors can display an empty name. See [middle compatibility](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/cli.go#L382-L393) and [later markers](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/cli.go#L457-L500). |
| [`scripts/install.sh` 276](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/scripts/install.sh#L268-L280) | Carry TODO | Machine installer does not install the CLI or create a `uc` alias. |
| [`scripts/uninstall.sh` 60, 68, 75](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/scripts/uninstall.sh#L54-L78) | Migration cleanup | All three markers remove pre-0.20 Corrosion service and binary handling after 0.22. |

### Equivalent non-implementation markers outside the 151-count

The following authored boundaries use runtime errors, headings, or support tables instead of TODO syntax. Generated gRPC
fallback implementations remain excluded.

- Linux is the daemon target. Darwin MTU, WireGuard, firewall, and Docker network controllers are explicit stubs
  ([MTU stub](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/network/mtu_darwin.go#L1-L10),
  [WireGuard stub](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/network/wireguard_darwin.go#L13-L30),
  [firewall stub](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/firewall/iptables_darwin.go#L1-L17),
  [Docker controller stub](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/controller_darwin.go#L1-L22)).
- `ExecOptions` exposes user, privileged, working directory, and environment fields under a `Not yet implemented`
  heading
  ([container.go lines 310-341](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/container.go#L310-L341)).
- Local machine initialization returns an explicit not-implemented error
  ([cli.go lines 183-191](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/cli.go#L183-L191)).
- The generic `ScheduleContainer` method is unimplemented. The active rolling strategy does its own concrete scheduling
  ([scheduler lines 27-54](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/scheduler/service.go#L27-L54)).
- WireGuard, Unix, and TCP connectors do not all implement proxy dialing
  ([WireGuard connector lines 83-85](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/connector/wireguard.go#L83-L85),
  [Unix connector lines 35-41](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/connector/unix.go#L35-L41),
  [TCP connector lines 44-50](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/connector/tcp.go#L44-L50)).
- Anonymous volumes and scaling to zero are rejected
  ([volume client lines 15-23](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/volume.go#L15-L23),
  [scale command lines 62-74](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/service/scale.go#L62-L74)).
- Compose published port ranges, external secrets, and external configs are rejected
  ([port conversion lines 92-107](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/port.go#L92-L107),
  [secret validation lines 147-187](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/project.go#L147-L187),
  [config conversion lines 21-33](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/config.go#L21-L33)).
- The Compose support matrix is the canonical broader list of unsupported and limited fields
  ([support matrix lines 13-76](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/8-compose-file-reference/1-support-matrix.md#L13-L76)).

## Implementation guidance for the Rust reconstruction

1. Create an `UPSTREAM_TODOS.md` ledger before implementation. Give every marker above a stable key, frozen source link,
   classification, Rust location, and status.
2. Copy behavior-affecting TODOs beside the Rust boundary when that boundary first appears. Include one sentence about why
   Uncloud accepts the limitation so future agents do not "repair" it casually.
3. Model weak states honestly. Examples include `MachineReachability::Suspect`, `Observed<T>`, `PartialDeployment`,
   `RollbackAttempt`, `ServiceIdentityConflict`, and `ContainerStatusTrust`. Better enums should expose uncertainty, not
   erase it.
4. Preserve one-shot command semantics. A Rust type called `Plan` or `DesiredService` must not imply a background
   reconciler.
5. Reuse Docker, WireGuard, Caddy, the hosted Uncloud DNS API, and a suitable replicated-state tool. Prefer a dependency
   that deletes owned code. Do not replace these with generalized internal frameworks.
6. Treat explicit ceilings as parity tests. Useful negative tests include no automatic reschedule after machine loss, no
   automatic global-service scale after machine addition, partial rolling-update persistence, A-only internal DNS, no
   machine-membership filtering in DNS and Caddy, plaintext resolved secrets, and local-only named volume data.

## Bottom line

Uncloud's simplicity comes from leaving coordination to the user and local recovery to Docker. Its reactive machinery is
narrow. Shared state projects container observations into DNS, Caddy, and peer configuration. It does not become a global
repair loop. The Rust rewrite should make partial, stale, conflicting, and unknown states clearer in types while retaining
the same operational weakness. Most accidental overbuilding will begin by trying to "fix" one of those states instead of
representing it.
