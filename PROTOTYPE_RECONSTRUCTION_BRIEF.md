# Ployz reconstruction brief

> **PROTOTYPE — owner approval required.** This is the concrete draft for
> [Draft and approve the reconstruction brief](https://github.com/getployz/ployz2/issues/12).
> It belongs on the throwaway `prototype/reconstruction-brief` branch until the
> owner approves or revises it. Do not treat it as an implementation decision or
> merge it to `main` before that ticket is resolved.

## Mission

Reconstruct Uncloud in Rust as **Ployz**: a runnable, maintainable product with
mostly the same public behavior and deliberately the same weak operational
semantics, expressed through Ployz-owned names and stronger local Rust types.

The reference is `psviderski/uncloud` `main` at
[`b7e224a1eff98813b1d1a32034d977be24be994e`](https://github.com/psviderski/uncloud/tree/b7e224a1eff98813b1d1a32034d977be24be994e).
Freeze that commit for all source, documentation, fixture, and test evidence.

This is a behavioral reconstruction, not a translation. Preserve product
outcomes, domain meaning, architecture, technology commitments, failure
semantics, and intentional non-implementation. Do not copy Go package layout,
types, control flow, schemas, protobufs, or wire formats. Ployz does not
interoperate with Uncloud clusters, daemons, configuration, or stored data.

When evidence disagrees, use this authority order:

1. explicit decisions on the
   [reconstruction map](https://github.com/getployz/ployz2/issues/1);
2. stated Uncloud architectural intent at the frozen baseline;
3. observable product and CLI behavior;
4. published behavior and command documentation;
5. current implementation detail.

The shortest faithful implementation minimizes mechanisms, not semantic
distinctions. Prefer a precise enum to branches around invalid states. Do not
turn that precision into consensus, reconciliation, fencing, transactions, or
automatic repair.

## Product identity and scope

Use Ployz-owned names everywhere the project controls:

- product and CLI: `ployz`;
- daemon: `ployzd`;
- environment variables: `PLOYZ_*`;
- configuration paths and keys, runtime names, service names, labels, injected
  variables, and release artifacts: Ployz names;
- no deprecated Uncloud aliases.

The CLI supports Linux and macOS and Windows through WSL. The daemon supports
Linux on AMD64 and ARM64. Ubuntu and Debian are tested server platforms; other
Linux distributions are best-effort. Native Windows and non-Linux daemon
targets are not part of the reconstruction.

Preserve automated remote provisioning through `machine init` and `machine
add`, but install Ployz artifacts. Local machine initialization remains
unimplemented. The CLI may connect through SSH, TCP, or a Unix socket and may
try an ordered set of entry connections. System `ssh` is the only SSH
implementation; drop the historical `ssh_go` and `ssh_cli` configuration
fields.

Keep using the hosted DNS API at `https://dns.uncloud.run/v1` and accept its
Uncloud-branded assigned domain as an opaque external value. Put this beside
that integration:

```rust
// TODO: Replace dns.uncloud.run and Uncloud-branded domains with
// Ployz-hosted DNS once that infrastructure exists.
```

Do not rebuild Uncloud-operated infrastructure. Do not implement the hidden
documentation-generator command. Do not read or migrate Uncloud configuration.

## Public behavior to preserve

The public surface is the complete documented product, not only bootstrap and
deploy. Preserve these workflows while allowing the command tree to evolve
under the deviation rule in the verification section.

| Area | Required behavior |
| --- | --- |
| Cluster membership | Remote `machine init`, `machine add`, `machine rm`, list, rename, update, RTT, and machine logs; provisioning, reset/no-reset behavior, fallback entry creation, and Caddy/DNS bootstrap options. |
| Local contexts | Create and mutate Ployz configuration, choose a current context, show/list/use contexts, edit ordered connections interactively, honor direct connection and context overrides, and fall back to the local daemon socket where applicable. |
| Services | `run`, Compose `deploy`, build, list, container list, inspect, scale, start, stop, remove, exec, merged logs, and local proxy; keep grouped and convenient root-level entry points where they remain in Ployz's declared command tree. Scaling to zero remains unsupported. |
| Images | List images and push a local image directly to all or selected machines, including platform selection. |
| Volumes | Create, list, inspect, and remove machine-local Docker volumes with machine selection, driver options, labels, force, quiet, and confirmation behavior. |
| Ingress | Deploy and inspect Caddy configuration and logs; preserve generated Caddyfile and JSON behavior, host ports, HTTPS, and certificate handling through Caddy. |
| Managed DNS | Reserve, show, and locally release a hosted domain; update wildcard records from observed public Caddy instances. Release continues to forget locally without deleting the hosted domain. |
| Networking inspection | Show WireGuard configuration and peer state. Do not implement a working client-side WireGuard connector in this effort. |
| General CLI | Version output, completion generated by Clap, prompts, configuration and environment precedence, aliases, cancellation, and human-readable output. Exact Cobra text and presentation are not compatibility targets. |

Renaming is required: for example, container-injected machine and hook markers
use Ployz-owned names rather than `UNCLOUD_*`. Preserve their meaning.

Ployz is free to choose a different Clap command shape. Keep a free-form
command-deviation ledger with one line per difference from the 58 frozen
generated command-reference pages and a reason. A declared difference is
allowed; silent drift fails verification. Clap-native parsing, help,
diagnostics, and completion are correct. Do not build a Cobra compatibility
layer.

## Compose contract

Use `docker compose config` to load and normalize Compose YAML, then parse its
output. The Docker Compose plugin is therefore a client-side prerequisite for
`ployz deploy`; it does not require a running local Docker daemon. Preserve
environment interpolation, `.env` behavior, `COMPOSE_FILE`-style discovery
under the appropriate Ployz/Compose inputs, profile selection, multiple files,
and the absence of a project prefix on service and volume names.

Support these standard areas:

- image and process: `build`, `command`, `entrypoint`, `image`, `init`,
  `pull_policy`, `stdin_open`, `tty`, and `user`;
- environment and files: `configs`, `env_file`, and `environment`;
- permissions: `cap_add`, `cap_drop`, `devices`, `gpus`, host `pid`,
  `privileged`, `sysctls`, and `ulimits`;
- resources: CPU, memory limit/reservation, shared memory, and the supported
  deploy reservations;
- health and shutdown: `healthcheck` and `stop_grace_period`;
- logging, including the default local Docker driver;
- bind, named-volume, and tmpfs mounts;
- replicated/global deploy mode and replica count;
- file and inline configs;
- the supported portions of `depends_on`, ports, secrets, and
  `deploy.update_config`.

Preserve Ployz's six extensions: `x-ports`, `x-caddy`, `x-machines`,
`x-command`, `x-context`, and `x-pre_deploy`. Secret commands execute in the
local deploy process. Resolved secrets remain plaintext in distributed service
state and Docker container configuration.

Preserve the observable warning-versus-error split. Unsupported service keys
such as custom DNS, links, labels, custom networks, and unsupported deploy
fields warn where the baseline warns and loading continues. Relative bind
mounts, external configs or secrets, unsupported secret providers, conflicting
standard and extension port syntax, and published port ranges remain hard
errors. Do not "complete" Compose support during the reconstruction.

## Domain model

Use the following meanings regardless of eventual crate or type names. The
canonical glossary on `main` is `CONTEXT.md`.

- A **Cluster** is the mesh as observed through one entry Machine. It is not an
  authoritative entity or complete global view.
- A **Machine** has a durable opaque Machine ID and a mutable, potentially
  ambiguous Machine Name. Its own Local Machine Phase (`uninitialized`,
  `joining`, `participating`, `resetting`) is separate from another Machine's
  Membership Observation (`unknown`, `up`, `suspect`, `down`).
- A **Service** is an observer-derived grouping of managed containers. There is
  no persisted service row, authoritative service aggregate, canonical current
  specification, or desired state.
- A **Service Container** and a pre-deploy **Hook Container** are distinct.
  Their durable runtime identity is a Container ID; generated Docker names are
  display values.
- A **Requested Service Spec** is normalized deploy input. Each created
  container carries its historical **Resolved Service Spec**. Containers in one
  observed Service may legitimately carry different specs.
- A **Deploy** is one bounded command attempt. Its **Deploy Plan** is an
  ephemeral ordered sequence calculated from an observer-relative snapshot.
  Its **Deploy Outcome** retains the completed prefix, failed operation,
  unexecuted suffix, and any narrow replacement compensation.
- A **Docker Volume** is identified by Machine plus Docker volume name. It is
  local storage and a possible placement anchor. A service-local volume
  reference, bind mount, tmpfs mount, and future managed ZFS volume are
  different concepts.
- A **Live Observation** comes from directly querying a Machine. A
  **Replicated Observation** comes from the local eventually convergent store.
  Neither implies completeness, freshness, or global authority.
- A **Partial Result** retains successful values alongside target-specific
  errors or omissions. It is expected and is not an atomic operation failure.
- **Name Ambiguity** preserves zero, one, or many matches. Do not choose, merge,
  or repair a winner in the domain model.

At minimum, keep Machine, Service, and Container identities distinct from one
another and from their names. Also distinguish machine subnet, management
address, machine gateway, container address, advertised endpoint, and
observer-selected endpoint. Model Docker lifecycle and health with explicit
variants and an unknown external fallback. Model bind, Docker-volume, and
tmpfs mounts as disjoint variants. Avoid typestate for remote lifecycles;
validated values and snapshot enums describe the real system more honestly.

The model must admit, rather than erase:

- incomplete Machine, Service, Container, and volume observations;
- duplicate names, subnets, addresses, keys, allocations, and Service IDs;
- mixed runtime states and historical specs within one observed Service;
- global Services missing containers until the user deploys again;
- completed operation prefixes, failed operations, and unexecuted suffixes;
- a failed replacement whose new stopped container remains for inspection;
- a failed stop-first compensation with both old and new containers stopped;
- stale replicated container observations after crash or partition;
- same-named but distinct Docker volumes on different Machines;
- removed but unreset Machines that retain containers and data.

## Runtime architecture

Ployz is a quorum-free set of equal, machine-local control loops, not one
logically serial control plane.

1. Local daemon state is authoritative for that Machine's own identity and
   network configuration.
2. Local Docker is authoritative for container runtime state on that Machine.
3. A machine-local SQLite database holds the complete Resolved Service Spec
   attached to each local managed container.
4. A local Corrosion replica holds redacted, eventually convergent observations
   of Machines and Containers. Services are derived from Container labels and
   observations rather than stored independently.
5. Membership is a local liveness judgment layered over replicated Machine
   records.
6. Internal DNS and Caddy configuration are asynchronous projections of
   replicated Container observations.

Each daemon writes to its local Corrosion API. Corrosion gossips CRDT-backed
SQLite changes and runs membership. Reads through different entry Machines may
disagree. Administrative requests enter through one reachable Machine, execute
there, target one Machine, or fan out to a set resolved from that entry's local
view. Remote calls are proxied directly. Fan-out can partly succeed.

Keep the transport gRPC-shaped with `tonic`, but carry serde-encoded Ployz
domain values in an opaque payload. No protobuf compatibility or generated
payload layer is required. The transparent proxy forwards one stream or merges
framed streams from multiple Machines while injecting Machine identity; it
must not learn the domain schema.

Joining has a catch-up barrier: the existing member captures its current
per-actor Corrosion versions, the joiner waits to reach them and to see no
locally known gaps, then starts dependent components. The wait can be
indefinite. It is not proof of global completeness and does not serialize
concurrent allocation.

## Networking

Preserve Docker, WireGuard, the equal-Machine full mesh, the default
`10.210.0.0/16` cluster container network divided optimistically into one
IPv4 `/24` per Machine, deterministic IPv6 management addressing, and direct
untranslated routing to Container addresses.

Each Machine owns a bridge subnet and gateway and publishes routes to peers.
Machine addition chooses from the locally observed free subnets without a
reservation or fence, so concurrent partitions may choose the same subnet.
Peer and endpoint updates are imperative and best-effort. Different observers
may select different endpoints for the same target.

NAT traversal is limited to advertised endpoint candidates, keepalives,
WireGuard roaming, and local endpoint rotation. There is no relay or
coordinated hole punching; at least one side of each pair must be reachable.
Do not add centralized IPAM, a topology controller, an overlay, relays,
security groups, ACLs, or a firewall portability layer.

The preserved structural ceilings are quadratic full mesh, at most 256
distinct `/24` allocations inside the default `/16`, broad trusted-mesh
access, no relay connectivity, potentially indefinite join catch-up, Linux
daemon scope, and a qualitative small-cluster/low-churn assumption. Do not
invent numeric support claims or convergence SLAs.

## Deployment and failure behavior

A deploy reads an observer-relative snapshot, plans once, and executes an
ordered finite sequence. It stops at the first failed operation. Successful
earlier operations remain. There is no persisted deployment resource,
transaction log, retry worker, automatic resume, general rollback, or
desired-state controller.

A replicated Service places the requested number of containers across the
currently eligible observed Machines, subject to placement and local-volume
constraints. A global Service plans one regular container on every currently
eligible observed Machine. Global mode is not a standing invariant: adding a
Machine does nothing until a user deploys again.

Replacement order is explicit when requested; otherwise host-port conflicts
and a single replica using a named Docker volume select stop-first. Other cases
use start-first. A new container is created, started, and monitored unless the
user skips health monitoring. Docker health informs routing and deploy
progress, but Ployz never reschedules or replaces an unhealthy container on
another Machine.

If a replacement fails health monitoring, stop and retain the new container
for inspection. For stop-first only, try to restart the old container if it
was running. Record whether that narrow compensation succeeded. Stop the rest
of the plan either way. Do not undo earlier successful replacements.

A pre-deploy Hook Container runs only when a Service plan will run or replace a
container. It uses the Service image and most configuration but disables
restart, health check, and published ports. A nonzero exit, timeout, or
cancellation stops the plan and retains the failed hook for inspection. Hook
success is not persisted, so a later deploy runs it again.

Direct start, stop, remove, inspect, logs, and volume operations fan out from
the selected entry's current view. Preserve successful siblings when another
target fails. Do not model an authoritative `Running` or `Stopped` Service
state.

## Mandatory weak semantics

These outcomes are parity requirements, not cleanup opportunities:

- reachable partitions continue local reads, writes, and administration with
  no quorum gate;
- no linearizable, monotonic, quorum-read, or cross-entry read-your-writes
  guarantee;
- snapshot uniqueness checks, name resolution, subnet allocation, scheduling,
  and placement have no locks, leases, reservations, revalidation transaction,
  or fencing;
- storage convergence may retain domain contradictions and never chooses a
  canonical winner;
- join, remove, deploy, scale, and fan-out may leave durable partial effects;
- Docker restarts locally; Ployz does not relocate, reschedule, rebalance, or
  autonomously repair;
- DNS and Caddy remain membership-blind and may retain stale but healthy
  upstreams;
- start-first may briefly interrupt a single replica while Caddy catches up;
- Caddy keeps its last known good configuration when a new load fails;
- internal DNS remains A-only with TTL zero;
- Docker volumes remain local and unreplicated;
- resolved secrets and managed-DNS credentials remain plaintext in their
  current stores;
- Cluster membership remains the authorization boundary and the mesh remains
  broadly trusted.

Local validation, diagnostics, typing, and error reporting may improve if they
preserve accepted states and distributed outcomes. Earlier rejection of the
same invalid local input is acceptable. New global rejection, winner
selection, fencing, reconciliation, repair, all-or-nothing execution, or
stronger security is not.

## Technology bindings

Pin external component versions to the frozen baseline or the explicit value
below. Prefer running mature components and speaking their APIs over owning
equivalents.

| Concern | Required Ployz choice |
| --- | --- |
| Replicated store | Corrosion as its pinned Docker container; own only HTTP and Unix-socket admin clients. |
| Ingress | Official Caddy image, exact `2.x.y` tag; use `/adapt` and the admin API. |
| Direct image push | `ghcr.io/psviderski/unregistry:0.4.1` as a sidecar with the containerd socket and published port; preserve the current skip-with-warning conditions. |
| Container runtime | Docker through `bollard`. |
| Compose | Shell out to `docker compose config`; parse normalized YAML with `serde_norway`. |
| RPC | `tonic` transport with serde-encoded domain types in an opaque payload. |
| Machine proxy | Minimal owned byte-level transparent proxy. |
| Daemon WireGuard/networking | `defguard_wireguard_rs`. |
| Client transport/provisioning | System `ssh` only. |
| Local SQLite | Synchronous `rusqlite` with `bundled`, called through `spawn_blocking`. |
| Corrosion HTTP | `reqwest` with auth and retry layers. |
| Corrosion admin socket | `tokio-util` `LengthDelimitedCodec` plus `serde_json`. |
| Internal DNS | `hickory-server` and `hickory-resolver`. |
| Registry references and access | `oci-client` and its `Reference` type. |
| Caddy DTOs | Hand-written serde structs for only the generated subset. |
| Terminal | `anstyle`/`anstream`, `dialoguer::Confirm`, `indicatif`, `crossterm`, and `tabled` only where the behavior needs them. Plain Caddy config output; no syntax-highlighting dependency. |
| Notifications and metrics | `sd-notify` or a minimal direct `$NOTIFY_SOCKET` write; `prometheus` for metrics. |
| Small utilities | `ipnet`, `shell-words`, `humansize`, `HashSet`, `sha2`, `oci-spec`, `toml`, `semver`, `uuid`, `backon`, serde, Clap, standard assertions, and `pretty_assertions` as their concrete needs appear. |
| CDI validation | About ten owned lines for the qualified-name check; no CDI crate. |

Do not port the unreachable client-side userspace WireGuard tunnel. Preserve
x25519 key parsing/formatting for WireGuard inspection and keep `Connector` as
a trait so a future Ployz WireGuard connector can be added. Put an adjacent
TODO at that boundary. Implementing the connector is a separate effort.

Ployz-owned integration code has a deliberate floor of only:

1. the transparent one-to-one/one-to-many Machine proxy;
2. the Compose unsupported-feature warning/error classifier and the six Ployz
   extensions;
3. Caddyfile and JSON config generation for the used subset.

Do not create abstractions for hypothetical alternate stores, runtimes,
transports, schedulers, ingress controllers, or orchestrators. Add a dependency
only when the standard library, the platform, and an already selected
dependency do not cover the concrete behavior.

## TODO and omission ledger

Before implementing behavior, create one `UPSTREAM_TODOS.md` ledger accounting
for all 151 authored TODO-style markers at the frozen baseline and the
equivalent non-TODO omissions. The exhaustive evidence inventory is the report
at
[`96b49f6`](https://github.com/getployz/ployz2/blob/96b49f60196abbd7de8eeb9ceec99a2f124d87e2/docs/research/omissions-todos-and-operational-boundaries.md).

Every entry needs a stable key, immutable upstream source link, one disposition
(`preserve boundary`, `carry TODO`, `resolve by Rust structure`, `migration or
Go cleanup / not applicable`, or `reference only`), eventual Rust location,
and status. Eliminated Go packages and dependencies still get a disposition;
do not create dead Rust code just to hold their comments.

When a behavior-affecting boundary appears in Rust, put an adjacent TODO there
that states the accepted limitation and links the ledger key. Each
implementation pull request links the affected keys and says whether it
preserves, exposes, or explicitly supersedes the weakness.

The highest-risk boundaries to retain include:

- no consensus, minority rejection, or fence for Machine admission;
- no drain or unschedulable phase during Machine removal;
- no local Machine initialization;
- no automatic global-Service reconciliation or failed-Machine rescheduling;
- no general deployment rollback or durable resume;
- no consistency check between an existing Service ID and a supplied Service
  Name during container creation;
- partial replicated rows and unreachable Machines may be omitted from current
  views — incomplete replicated data must stay tolerable and must never be
  presented as authoritative completeness, but the current behavior of silently
  omitting those rows is explicitly *not* frozen and may be changed;
- placement ignores image-platform support, local image presence for
  `pull_policy: never`, and memory reservation;
- mutable Docker resource changes are classified but recreated rather than
  updated in place;
- ingress-port-only changes recreate containers;
- start-first does not wait for Caddy projection before stopping the old
  container;
- Compose dependency conditions and scheduling from resolved specs remain
  incomplete;
- Ployz does not pull images from other Cluster Machines to satisfy pull
  policy;
- DNS and Caddy do not filter by Machine membership;
- internal DNS lacks non-A answers and nonzero caching TTL;
- L4 Caddy routing is not implemented;
- Docker/firewalld interaction, network-recreation invalidation, broad embedded
  registry reachability, and large-Cluster bootstrap remain known ceilings;
- managed DNS tokens stay plaintext and release remains local-only.

Do not implement these TODOs merely because Rust makes a fix convenient.

## Executable parity contract

Never run or ship an upstream Uncloud binary as a test oracle. No Go toolchain,
live differential test, or captured upstream golden belongs in developer or CI
paths. Transfer assertions and fixtures, not Go code or source structure.

Use three layers:

1. **Semantic unit tests.** Re-express the roughly 85 meaningful cases from
   frozen `pkg/client/compose`, `pkg/client/deploy`, `pkg/api`, Caddy, DNS,
   log-merging, configuration, and connection tests. A subsystem is not done
   until its corresponding cases pass; do not port tests before code exists.
2. **Command shape.** Walk the Clap command tree in-process and compare command,
   alias, flag, default, and environment annotations with data derived from the
   58 frozen generated CLI reference pages. Apply the free-form deviation
   ledger; undeclared differences fail.
3. **Cluster end-to-end.** Build a Ployz-owned Docker-in-Docker harness: one
   network per test Cluster, privileged Machine containers running `ployzd`,
   WireGuard tools, a preloaded Corrosion image, preallocated host ports,
   first-Machine init, subsequent joins, bounded readiness waits, and teardown.
   Publish a multi-architecture test image and re-express the meaningful frozen
   end-to-end scenarios.

Copy these frozen fixture inputs verbatim:

- `compose-basic.yaml`;
- `compose-multi-service.yaml`;
- `compose-placement.yaml`, `compose-placement-comma.yaml`, and
  `compose-placement-nonexistent.yaml`;
- `compose-volumes.yaml` and `compose-global-volume.yaml`;
- `compose-configs.yaml` and `configs/test-config.conf`;
- `compose-predeploy.yaml`;
- the `compose-build-basic/` tree.

All parity assertions obey six tolerances:

1. wait for eventual outcomes within a bound; never freeze an incidental timer
   or backoff;
2. compare unordered collections as sets;
3. assert failed operations and Partial Result composition, not a formatted
   joined error string;
4. preserve ambiguity but never assert which duplicate name or identity wins;
5. do not assert help prose, prompt wording, diagnostics, layout, color, or
   progress rendering;
6. normalize IDs, timestamps, generated container names, and dynamically
   assigned ports.

Keep a small shared test-support module with normalization, set comparison, and
bounded-eventually helpers. Tolerance never weakens the distributed outcome:
operation order in the completed Deploy prefix, persisted partial effects, the
failed operation, the unexecuted suffix, and the absence of repair machinery
remain exact.

The cluster suite must cover these negative-parity families:

1. partition-local reads, writes, and administration without a quorum gate;
2. partial Deploy persistence with completed prefix and unexecuted suffix;
3. no autonomous rescheduling, relocation, or replacement;
4. contradictory names, subnets, addresses, keys, and specs remain
   representable after convergence;
5. membership-blind DNS/Caddy, possible brief propagation interruption, and
   last-known-good Caddy behavior;
6. Machine-local volume data and placement anchoring.

Run semantic and command-shape layers on every pull request. Run the cluster
layer after merge to `main` and nightly. Retry a failed cluster scenario once;
two failures fail the gate rather than being dismissed as a flake.

## Definition of a faithful reconstruction

Ployz is complete when:

- the preserved product workflows operate under Ployz-owned naming on the
  supported platforms;
- every approved product change and exclusion above is reflected explicitly;
- local types express identity, provenance, ambiguity, partial outcomes, and
  external/unknown states without implying stronger guarantees;
- Corrosion, Docker, WireGuard, Caddy, Unregistry, hosted DNS, and the chosen
  Rust bindings meet their fixed boundaries;
- all 151 TODO-style markers and equivalent omissions have ledger dispositions,
  with behavior-affecting TODOs adjacent to their Rust boundaries;
- the three verification layers and six negative-parity families pass under
  the approved tolerances;
- implementation review finds no Go-structure mirroring and no unapproved
  consensus, fencing, reconciliation, transaction, repair, rescheduling,
  rollback, security, or compatibility machinery.

## Deliberately unresolved after this brief

This brief fixes behavioral and architectural boundaries but does not choose:

- workspace, component, or crate layout;
- implementation tickets or their dependency order;
- packaging, installation, cross-compilation, release, migration, and
  operational validation details;
- policy for considering Uncloud changes after the frozen baseline.

Those are the next decisions on the reconstruction map. In particular,
`rusqlite`'s bundled C source means AMD64 and ARM64 builds need an appropriate
cross-compiling C toolchain, unlike the pure-Go baseline. Do not let an
implementation agent silently choose `cross`, `cargo-zigbuild`, packaging
formats, or a release policy before that decision is made.
