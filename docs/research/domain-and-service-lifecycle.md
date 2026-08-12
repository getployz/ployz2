# Uncloud domain model and service lifecycle

## Scope and baseline

This report extracts the domain model that the Rust reconstruction needs to preserve. It covers machines, services,
containers, deployments, and volumes. It separates domain meaning from Go and protobuf representation. It does not
propose stronger consistency, coordination, reconciliation, or recovery guarantees.

The sole reference is Uncloud commit
[`b7e224a1eff98813b1d1a32034d977be24be994e`](https://github.com/psviderski/uncloud/tree/b7e224a1eff98813b1d1a32034d977be24be994e).
The design document is useful evidence of intent, while source and tests define the behavior at this baseline.

## Executive model

Uncloud has three kinds of state with deliberately different authority:

1. A machine owns its identity, key material, network allocation, and mutable advertised properties in a local state
   file. It republishes its own information into the distributed store.
2. Docker owns the actual local container and volume state. A small local SQLite record attaches the full service spec to
   each managed container. The distributed store receives asynchronously refreshed observations of those containers.
3. The CLI owns deployment intent only for the lifetime of a command. It reads a snapshot, constructs a finite ordered
   plan, executes the plan directly against target machines, and stops on the first error. No persistent deployment or
   desired-service controller continues the work afterward.

This split follows the stated choice to favor imperative operations. Commands call a target directly so errors remain
predictable. Shared state drives projections such as DNS and ingress, but not continuous service scheduling. Docker
restarts a container on its current machine. Uncloud does not move it elsewhere, and scheduling happens only when a user
issues a deployment command ([design intent](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/misc/design.md#L99-L122)).

The central reconstruction rule is therefore:

> Stronger types may make a local snapshot, command, transition, or result unambiguous. They must not imply that a
> service has a single authoritative desired state, that a plan is transactional, or that the cluster continuously
> reconciles workloads.

## Concept and relationship map

| Concept | Identity | Authority and lifetime | Main relationships |
| --- | --- | --- | --- |
| Cluster | No explicit cluster ID at this baseline | Distributed network configuration and machine records | Contains registered machines and a cluster network |
| Machine | Random machine ID, plus a mutable human name | Local machine state is authoritative for that machine | Owns one subnet, management IP, WireGuard key and endpoints. Hosts containers and volumes |
| Membership observation | Machine ID plus observer-local liveness result | Ephemeral result from the connected member | Qualifies a registered machine as unknown, up, suspect, or down |
| Service | Random 32-character hexadecimal service ID | Derived aggregate that exists only while managed containers with its ID exist | Groups regular and hook containers across machines |
| Service spec | No independent identity | Immutable historical value stored per container | Describes the container template, service mode, placement, ports, volumes, configs, hook, and update options |
| Service container | Docker container ID and generated human name | Docker is runtime authority. The local DB supplies its attached service spec | Belongs to one service and one machine |
| Hook container | Docker container ID | One-shot Docker container retained until a later deploy cleans it up | Belongs to a service but is separated from regular replicas |
| Deployment | No persisted ID | In-memory command object | Reads one cluster snapshot, resolves one service spec, and produces one plan |
| Plan | No persisted ID | Cached inside that command invocation | Ordered volume operations, then ordered service plans, then ordered container operations |
| Volume spec | Service-local reference name | Value in a service spec | Resolves to a bind mount, named Docker volume, or tmpfs |
| Machine volume | `(machine ID, Docker volume name)` in practice | Docker on that machine | Constrains placement of services that need the volume |

Machine IDs come from the same random ID generator used for service IDs. The service API explicitly validates exactly 32
lowercase hexadecimal characters, while Docker container IDs remain Docker identities
([machine ID generation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/machine.go#L11-L14),
[service ID validation and generation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/service.go#L43-L50),
[new service plan](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/strategy.go#L505-L525)).

## Machines

### Meaning and identity

A registered machine has a stable ID, mutable name, WireGuard network configuration, optional public ingress IP, and
runtime metadata such as daemon version and platform. Its network configuration contains a unique machine subnet,
management IP, endpoints, and public key
([machine protocol](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/api/pb/machine.proto#L34-L59)).
The local `machine.json` state also holds the private key, minimum store version needed during a join, and the local
Corrosion API token. The source explicitly calls this machine-specific state the source of truth
([local state](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/state.go#L20-L41),
[local update ownership](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L1107-L1127)).

A machine name is a human selector, not the durable identity. APIs accept either name or ID. Names are checked for
uniqueness against the connected member's current store view, but the architecture does not fence concurrent writes in
partitions
([lookup](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/machine.go#L17-L30),
[rename check](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L1130-L1183),
[deliberate missing consensus](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/cluster.go#L163-L180)).

The join token is a descriptor rather than an authority-bearing identity. It contains the candidate's public key,
discovered public IP, and reachable endpoints as base64-encoded JSON. The Rust model should not imply that this token is
an authentication secret
([token representation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/token.go#L13-L56)).

### Lifecycle

The persistent machine lifecycle is small. Registration and local join are separate operations:

```text
Uninitialised, key pair already generated
  -> initialise a new cluster OR get registered by an existing member
Registered in the cluster snapshot but not necessarily joined
  -> accept a prepared join locally
Registered locally, with ID and allocated network
  -> wait for the joining store version and known gaps
Participating
  -> optionally update own advertised name, public IP, or endpoints
  -> reset, which schedules shutdown and removes local cluster data
Uninitialised after daemon restart
```

The code treats a non-empty machine ID as the definition of “initialised.” Initialisation and joining are mutually
exclusive and each closes the one-shot `initialised` signal after atomically saving the assigned local state
([initialised predicate](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L326-L338),
[initialise transition](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L761-L875),
[join transition](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L878-L980)).
A failed local join can leave a registered row for a machine that never accepted its configuration because registration
happens before the CLI invokes `JoinCluster`
([two-phase add](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/cli.go#L455-L493)).
A joining machine may expose its mesh API before it is ready for store-dependent cluster operations. It waits until its
local store reaches the version captured from an existing member, then starts the remaining components and marks the
cluster ready
([join barrier](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster.go#L176-L205),
[barrier completion](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster.go#L369-L437)).

Reset is an asynchronous transition. The RPC only marks `resetting` and cancels the daemon. Shutdown later cleans up
local cluster resources and persistent state. Repeated reset requests fail while the first reset is underway
([reset](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L1305-L1327),
[shutdown cleanup](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L577-L596)).

Cluster removal and local reset are separate operations. Removing a machine deletes its container observations and
machine record from the connected partition's store. Reset clears the target machine. There is no draining or
unschedulable state, so scheduling can race removal. A removal can deliberately happen without a successful reset, so
stale runtime resources may remain on that host
([missing drain state](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/machine/rm.go#L98-L130),
[cluster removal](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/cluster.go#L244-L269)).

### Membership is an observation, not a machine lifecycle state

Membership has four values: `Unknown`, `Up`, `Suspect`, and `Down`. A suspect member is deliberately treated as up until
it either refutes suspicion or becomes down. The current request-serving machine is always reported as up
([membership semantics](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/api/pb/cluster.proto#L32-L47),
[membership derivation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/cluster.go#L198-L241)).
Scheduling defines “available” as anything other than down, so suspect, and technically unknown, remain eligible
([available filter](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/machine.go#L64-L72)).

The Rust model should keep local readiness and membership separate. A useful representation is a `LocalMachinePhase`
enum for mutually exclusive local phases such as uninitialised, joining/catching-up, participating, and resetting, plus a
separate `MembershipObservation` enum. This makes existing states explicit but must not turn one member's liveness
observation into authoritative cluster truth.

## Services and service specs

### A service is a derived aggregate

There is no services table and no independently persisted service record. The distributed schema stores machines and
containers. Service ID and name are generated columns extracted from container labels
([distributed schema](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/store/schema.sql#L10-L42)).
`InspectService` broadcasts to currently available machines, collects managed Docker containers, resolves ID before name,
then builds a service aggregate from the matching containers. No containers means “service not found”
([inspection](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/service.go#L88-L188)).

This has important semantics:

- Creating the first container creates the observable service. Removing the last regular and hook container removes it.
- A service has no lifecycle state independent of its containers.
- A service may be partially observed when a machine is down or a broadcast fails.
- Concurrent creation can produce different service IDs with the same name. Name lookup detects ambiguity when it sees
  it and asks for the ID. It does not prevent it with consensus
  ([ambiguity handling](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/service.go#L148-L167)).
- The distributed-store inspection path is explicitly stale and still has TODOs for untrusted sync status and duplicate
  names
  ([store inspection warning](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/service.go#L190-L210),
  [store ambiguity TODOs](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L1330-L1348)).

`ServiceSpec` is called desired state in Go, but the domain does not have one current cluster-wide service spec. The full
resolved spec is stored once per container in the host's private SQLite database. A rolling update can therefore expose
containers with different historical specs at the same time
([local container-spec record](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/db.go#L28-L43),
[create and attach spec](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/server.go#L743-L758),
[service endpoint comment](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/service.go#L570-L576)).
In Rust, the precise name should communicate this, for example `ContainerServiceSpec` for the persisted resolved value
and `RequestedServiceSpec` for command input. Do not introduce a canonical `CurrentServiceSpec` unless the source gains
one.

### Service identity and invariants

The service ID is stable across deployments. The first plan generates it and later plans reuse the observed service ID.
The service name and mode cannot change after the first deployment
([plan identity](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/strategy.go#L505-L525),
[deployment validation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/deploy.go#L334-L367)).
Service names are DNS labels of at most 63 characters. Mode is either replicated or global. The default is replicated,
and a replicated zero replica count defaults to one
([spec defaults and validation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/service.go#L119-L157)).

The Go representation weakens these closed choices into strings. Rust can preserve behavior with enums:

- `ServiceMode::{Replicated { replicas: NonZeroU32 }, Global}`. This prevents the meaningless combination of global
  mode with a replica count.
- `UpdateOrder::{StartFirst, StopFirst}` with `Option<UpdateOrder>` only in unresolved input, where absence means derive
  the order.
- `PullPolicy::{Always, Missing, Never}`.
- `PortPublication::{Ingress { hostname, load_balancer_port, container_port, http_protocol }, Host { bind,
  published_port, container_port, transport_protocol }}`. The source currently uses independent strings and optional
  fields, then validates invalid combinations at runtime
  ([port validation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/port.go#L21-L90)).

These enums encode existing validated variants. They do not add a service controller or stronger global uniqueness.

## Containers

### Identity, ownership, and kinds

A service container is a Docker container plus its attached resolved service spec. Docker labels establish that it is
Uncloud-managed and record service ID, name, mode, ports, and optional hook kind
([labels and aggregate](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/container.go#L17-L35),
[service-container type](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/container.go#L171-L207)).
Regular containers get names `<service>-<random four characters>`. Pre-deploy containers add the hook kind to the name.
Both use Docker's container ID as durable runtime identity
([client-side naming](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/container.go#L65-L98)).

The closed domain choice is `ContainerKind::{ServiceReplica, PreDeployHook}`. The protobuf and label representations use
an enum in one place and string presence in another. Rust should normalize them into one enum at the boundary. It should
not create a general job system. The source explicitly says batch jobs and one-off containers are future service modes
([restart policy comment](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/server.go#L643-L665)).

### Runtime state and health

Container lifecycle is Docker's lifecycle, not an Uncloud-owned state machine. The current API embeds Docker's inspect
response, which allows overlapping booleans and a free-form status string. Uncloud derives health as follows:

- not running, paused, or restarting means unhealthy.
- running with no enabled health check means healthy.
- running with a health check means healthy only when Docker reports `healthy`.

Human display distinguishes running, paused, restarting, health-starting, removing, dead, created, and exited states
([health derivation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/container.go#L55-L81),
[display states](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/container.go#L83-L134)).

A Rust boundary adapter can translate a valid Docker snapshot into a sum type such as:

```text
ContainerRuntimeState =
  Created
  | Running { health: NotConfigured | Starting | Healthy | Unhealthy }
  | Paused
  | Restarting { last_exit_code }
  | Exited { exit_code, finished_at }
  | Removing
  | Dead
  | UnknownDockerState { raw }
```

The unknown case is important because Docker is the authority and can evolve independently. This enum should describe an
observation. It must not trigger an Uncloud reconciliation loop.

Regular service containers use Docker's `unless-stopped` restart policy. Hook containers disable restart and health
checks and remove published ports. This is the only automatic workload maintenance: Docker restarts on the same host
([regular and hook configuration](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/server.go#L660-L715)).

### Observation and sync status

Each machine watches local Docker events and also performs a 30-second fallback scan. It upserts current managed
containers and deletes observations no longer present locally
([Docker observation loop](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/controller.go#L17-L27),
[sync behavior](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/controller.go#L150-L194)).
The distributed record uses string states `synced` and `outdated`, but even `synced` can be stale after a crash or
partition. Current list queries only return synced records, while a TODO questions whether the field is needed
([record semantics](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/store/container.go#L18-L33),
[subscription TODO](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/store/container.go#L208-L219)).

If retained, Rust can model `ObservationFreshness::{SyncedAt(Instant), OutdatedSince(Instant)}`. It must still document
that “synced” is a local claim, not proof of current runtime state.

## Deployments and service lifecycle

### Deployment is a command, not a resource

A deployment has no ID, persisted state, owner, status endpoint, retry worker, or reconciliation loop. It contains a
requested spec, an optional currently observed service, a strategy, a captured cluster snapshot, and a cached in-memory
plan. Planning validates and resolves the spec, takes a current machine and volume snapshot, and asks the rolling strategy
for an ordered list of operations
([deployment model](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/deploy.go#L19-L48),
[planning](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/deploy.go#L268-L332)).

The word “reconcile” in method comments means “calculate the finite operations for this invocation.” It does not mean a
background reconciler. This distinction should be recorded beside the Rust planner so future work does not add machinery.

A Compose deployment is likewise ephemeral. It resolves all service specs, plans missing volumes, creates one service
plan per service, then executes all volume operations before all service operations
([Compose planning](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/deploy.go#L62-L118),
[plan execution](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/plan.go#L117-L129)).

### Finite operation algebra

The actual plan vocabulary is small:

- create a named Docker volume on one machine.
- create, start, and health-monitor a new service container.
- stop a container.
- stop and remove a container.
- replace a container start-first or stop-first.
- stop an old pre-deploy hook.
- run and wait for a new pre-deploy hook.
- sequence operations.

The Go interface hides these variants behind dynamic dispatch. A Rust `DeploymentOperation` enum is a direct model of
the existing algebra and is simpler to exhaustively format, execute, and test. It should contain target machine and
service/container IDs as typed newtypes. It does not need an extensible plugin interface because only the rolling
strategy exists at this baseline
([operation interface](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/operation/operation.go#L9-L22),
[rolling strategy interface and implementation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/strategy.go#L15-L43)).

Execution is intentionally non-transactional. Sequence execution stops at the first error and does not undo earlier
successful operations. The docs explicitly state that if the first replacement succeeds and the second fails, the first
stays in place and later containers stay untouched
([sequence](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/operation/sequence.go#L9-L20),
[documented partial result](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/4-guides/1-deployments/4-rolling-deployments.md#L155-L181)).

Rust should expose this truth in the result type. A suitable shape is a `PlanExecutionReport` containing completed
operations and either `Complete` or `Stopped { failed_operation, error, remaining_operations }`. This is stronger local
modeling of a result that already occurs. It must not add a transaction log, automatic resume, or cluster-wide rollback.

### Replicated service rules

A replicated service asks for a count of containers across currently available machines that satisfy placement and
volume constraints. The planner randomizes eligible machine order, then uses simple round-robin placement while
prioritizing machines that already have up-to-date containers. Non-running containers do not count as healthy replicas.
It runs missing containers, replaces mismatched ones, and removes extras
([replicated planning](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/strategy.go#L60-L206)).

The planner compares each container's attached historical spec with the requested spec. The result has three values:
`UpToDate`, `NeedsUpdate`, and `NeedsRecreate`. Most changes require recreation. Mutable Docker resources can be detected
as needing an in-place update, but execution currently treats that as recreation because in-place update is not
implemented
([spec-change classification](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/container.go#L12-L109),
[execution TODO](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/strategy.go#L169-L187)).
`ContainerSpecStatus` is already conceptually an enum and should become one in Rust. Preserve the `NeedsUpdate` TODO and
current recreate behavior rather than quietly implementing a new update mechanism.

### Global service rules

A global service means exactly one regular container on each currently available eligible machine. The planner repairs
duplicate or stopped containers only when a user runs deploy. Containers on newly joined machines do not appear
automatically. The user must run deploy again
([global planner](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/strategy.go#L209-L277),
[documented trigger](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/4-guides/1-deployments/3-deploy-global-services.md#L29-L32)).

This is an essential limitation. `Global` describes how one deployment calculates its target set. It is not a standing
one-per-machine invariant enforced by the cluster.

### Replacement and health rules

Replacement order is either explicit or derived:

1. explicit order wins.
2. conflicting host ports require stop-first.
3. a single-replica service with a mounted named Docker volume uses stop-first to avoid overlapping writers.
4. multiple replicas with volumes, bind mounts, tmpfs, and all other cases use start-first.

The planner assumes concurrent volume access is intentional when more than one replica is requested
([order algorithm](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/strategy.go#L418-L445),
[documented assumptions](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/4-guides/1-deployments/4-rolling-deployments.md#L22-L41)).

Running a new container is create, start, then health-monitor unless the caller skips monitoring. A container without a
health check must remain running for the monitor period. One with a health check may succeed early when healthy and
tolerates transient unhealthy status during the monitor period
([run operation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/operation/container.go#L28-L68),
[monitor contract](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/container.go#L392-L460)).

Replacement has narrowly local rollback. If the new container fails health monitoring, Uncloud stops it and preserves it
for inspection. For stop-first only, it tries to restart the old container if it had been running. It then returns an
error and stops the whole remaining plan. Start-first needs no old-container restart because the old container was not
stopped yet. Failures at other steps do not get general rollback
([replacement failure path](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/operation/container.go#L173-L258)).

The existing `ContainerHealthError` is a distinct domain failure with service, container, and machine context. Rust can
make all operation failures enum variants such as create failed, start failed, health failed with rollback outcome, stop
failed, and remove failed. This records actual partial outcomes more accurately. It must not retry or compensate beyond
the source behavior.

### Pre-deploy hook rules

A hook is planned only when a service has a hook and the service plan contains at least one run or replace operation. It
runs on the target of the first such operation. Old hook containers are stopped and cleaned up first. The new hook uses
the service image and most container configuration, but disables restart, healthcheck, and published ports. Exit code
zero continues the deployment. Non-zero exit, timeout, or cancellation stops it and the remaining deployment. The failed
container stays for inspection
([hook planning](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/strategy.go#L447-L503),
[hook execution](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/operation/predeploy.go#L70-L186)).

Hook success is not persisted as a deployment milestone. If a later container fails and the user retries the deploy, the
hook runs again. Hook commands should therefore be idempotent
([documented retry semantics](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/4-guides/1-deployments/5-pre-deploy-hooks.md#L211-L216)).

### Direct service commands

Start, stop, and remove service are not service-state transitions stored anywhere. Each command first observes the
service's containers, then calls container operations concurrently. It joins per-container errors and does not roll back
successful siblings. Start excludes old hook containers, while stop and remove include them
([remove](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/service.go#L213-L253),
[stop and start](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/service.go#L255-L319)).

Do not model `ServiceState::{Running, Stopped}` as an authoritative enum. A service can have an arbitrary mixture of
running, stopped, failed, inaccessible, and differently configured containers. A local summary may instead be a derived
`ServiceObservation` with per-container states and explicit completeness.

## Volumes

### Three different resources currently share one struct

`VolumeSpec` represents three mutually exclusive mount sources:

- bind mount: an absolute host path plus create and propagation options.
- named Docker volume: a Docker name, optional driver and labels, no-copy, and subpath.
- tmpfs: in-memory mount options.

The Go struct has a string discriminator and three optional option structs, so it admits invalid combinations before
validation
([volume variants](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/volume.go#L17-L65),
[validation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/volume.go#L96-L113)).
This is a direct Rust enum candidate: `VolumeSource::{Bind(BindMount), Named(NamedVolumeRef), Tmpfs(TmpfsMount)}`.

The service-local reference name and Docker volume name are different concepts. A volume spec can alias an existing
Docker volume. A mount points to the service-local name, while scheduling and Docker operations use the resolved Docker
name
([name resolution](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/volume.go#L67-L94),
[mount reference](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/volume.go#L195-L215)).
Use separate `VolumeRefName` and `DockerVolumeName` newtypes so aliases cannot be confused.

### Volumes are machine-local

A `MachineVolume` is explicitly a Docker volume paired with the machine that owns it. Create and remove always resolve
one target machine and call that machine's Docker daemon. Anonymous top-level volume creation is unsupported
([machine-volume model](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/volume.go#L235-L243),
[volume operations](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/volume.go#L15-L49),
[remove](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/volume.go#L103-L124)).
The same name on two machines denotes two physical Docker volumes. It is not replicated storage.
Commands that inspect by volume name therefore require a machine selector when more than one machine has that name
([ambiguous volume inspection](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/volume/inspect.go#L62-L79)).

Cluster-wide volume listing is also a partial observation. Per-machine failures are warned about and omitted from the
returned list rather than represented structurally
([partial volume listing](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/volume.go#L65-L100)).

Named volumes constrain placement. If a required volume exists, replicated services must run where a compatible instance
exists. A missing named volume for replicated services is created on one eligible machine. A volume used by a global
service is created on every eligible machine. Sharing one volume between global and replicated services is rejected
because those rules conflict
([volume scheduler contract](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/scheduler/volume.go#L12-L32),
[global versus replicated scheduling](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/scheduler/volume.go#L159-L250)).

Volume scheduling operates only on the command's captured `ClusterState`. It mutates the in-memory planned state so
later service plans know where volumes will be created. It does not reserve a volume location or coordinate competing
deployments
([snapshot state](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/scheduler/state.go#L12-L58),
[planned update](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/scheduler/volume.go#L253-L263)).

## Accepted partial and ambiguous states

The Rust design must be able to represent these states rather than erase them:

- a registered machine that is down, suspect, or unknown.
- a locally initialised machine whose network or cluster services are not ready yet.
- a joining machine waiting for a minimum replicated-store version.
- a service observation that is incomplete because one machine could not answer.
- two service IDs with the same human name after concurrent creation.
- a service whose containers carry different specs during or after a partial deployment.
- a service with mixed running, stopped, failed, and hook containers.
- a global service missing containers on newly added or unavailable machines until the next deploy.
- a plan with a successful prefix, one failed operation, and an unexecuted suffix.
- a failed replacement whose new stopped container remains for inspection.
- a failed stop-first rollback where both new and old containers are stopped.
- a locally synced container observation that became stale during a crash or partition.
- a named volume that exists on only one machine, or separate same-named volumes on multiple machines.
- a removed machine whose unreset host still has containers or volumes.

These are not all bugs. Several follow directly from imperative, AP, quorum-free operation. Modeling them with enums and
explicit result variants is valuable. Preventing them with locks, fencing, consensus, transactions, durable workflows,
or controllers would change the architecture.

## Recommended Rust type boundaries

These changes improve representation without changing behavior:

| Weak source representation | Rust boundary | Preserved limitation |
| --- | --- | --- |
| Machine readiness spread across empty ID, channels, and boolean | `LocalMachinePhase` enum | Phase is local only and says nothing authoritative about remote membership |
| Protobuf membership integer | `MembershipObservation` enum | Observation can be stale and differs by partition |
| Raw ID strings | `MachineId`, `ServiceId`, `ContainerId` newtypes | Names remain ambiguous human selectors |
| Service mode plus unrelated replica count | `ServiceMode` enum with replicated count | Global targets are recalculated only on user deploy |
| Service assembled as if complete | `ServiceObservation { completeness, containers }` | Missing machines remain missing, not repaired |
| Docker state flags and status string | `ContainerRuntimeState` enum with unknown fallback | Snapshot does not drive reconciliation |
| Hook detected by label presence | `ContainerKind` enum | Only service replica and pre-deploy hook exist |
| Pull and update strings | `PullPolicy`, `UpdateOrder` enums | Same pull and replacement behavior |
| One volume struct with discriminator | `VolumeSource` enum | Named volumes remain machine-local |
| One string for spec and Docker volume names | Separate name newtypes | Aliasing remains supported |
| Dynamic operation interface | `DeploymentOperation` enum | Operations still execute sequentially and stop on first error |
| `error` with implicit side effects | `PlanExecutionReport` and operation error enums | No transaction, durable resume, or general rollback |
| `ContainerSpecStatus` string alias | `SpecChange` enum | `NeedsUpdate` still recreates until its preserved TODO is addressed |

Avoid typestate that forces remote or long-running transitions into compile-time ownership chains. Machines and
containers change outside the process. Plain validated value types and snapshot enums are more accurate and maintain
less code.

## Relevant preserved TODO boundaries

The reconstruction-wide TODO inventory should retain these domain-relevant comments or their precise Rust equivalents:

- Machine admission does not announce and reach consensus and may proceed in a minority partition
  ([source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/cluster.go#L174-L176)).
- Machine removal has no draining or unschedulable phase, so new placement can race removal
  ([source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/machine/rm.go#L98-L99)).
- Container creation does not verify that service name is consistent with an existing service ID
  ([source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/container.go#L54-L59)).
- Failed machines are warned about but are not returned as structured list/inspect results
  ([source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/service.go#L121-L131)).
- Failed machines are likewise omitted rather than returned as structured volume-list results
  ([source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/volume.go#L65-L77)).
- Store-backed service inspection does not apply sync-status trust or resolve duplicate service names
  ([source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/machine.go#L1339-L1348)).
- Placement does not account for image platform support or `pull_policy: never`, and replicated planning does not limit
  targets to machines that already have the image
  ([constraints](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/scheduler/constraint.go#L22-L48),
  [replicated planner](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/strategy.go#L60-L64)).
- Memory reservation does not constrain placement
  ([source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/resources.go#L12-L23)).
- Mutable resource changes are classified but not updated in place
  ([source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/strategy.go#L169-L187)).
- Ingress ports live on container labels, so port-only changes recreate containers
  ([classification](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/container.go#L47-L56),
  [label TODO](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/server.go#L589-L600)).
- Start-first can still have brief downtime because Caddy learns about the new container asynchronously, and the
  deployer does not wait for that projection before stopping the old container
  ([source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/operation/container.go#L240-L249)).
- Compose `depends_on` conditions are not properly turned into service deployment ordering conditions
  ([source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/deploy.go#L101-L107)).
- Volume scheduling currently uses unresolved service specs
  ([source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/deploy.go#L130-L149)).
- The same deployment object can be run more than once
  ([source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/deploy/deploy.go#L370-L380)).
- Pull policies do not pull images from other cluster machines
  ([source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/service.go#L30-L40)).

These TODOs are evidence of known ceilings. They are not permission to add machinery during reconstruction. Each should
remain visible until a later explicit decision resolves it.

## Reconstruction guardrails

1. Do not add a persisted service row merely to make the Rust aggregate feel conventional.
2. Do not add a persisted deployment resource, worker, transaction log, retry queue, or resume protocol.
3. Do not convert the finite deployment planner into a background desired-state controller.
4. Do not make global mode automatically react to machine membership changes.
5. Do not reschedule containers from down machines. Docker only restarts locally.
6. Do not make service names globally unique with quorum, leases, or fencing.
7. Do not treat an available-machine or container snapshot as complete or current.
8. Do not turn local replacement rollback into whole-plan rollback.
9. Do not make named Docker volumes distributed or mobile.
10. Do use enums and newtypes where the source already has a closed set or an identity distinction.
11. Do preserve unknown/external states at Docker and distributed-store boundaries.
12. Do comment beside tempting extension points that partial outcomes and user-triggered scheduling are intentional.

The shortest faithful implementation is not the one with the fewest types. It is the one with the fewest mechanisms.
Precise enums can delete validation branches and ambiguity. They should not smuggle in a controller.
