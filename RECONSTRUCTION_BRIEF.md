# Ployz Reconstruction Brief

**Audience:** implementation agents building Ployz. You are expected to read this file plus the repo, and nothing else, before writing code.

**Baseline:** `psviderski/uncloud` `main` at `b7e224a1eff98813b1d1a32034d977be24be994e`. Frozen. Every upstream link in this brief is pinned to it.

**Status:** binding. Approved through [Draft and approve the reconstruction brief]. It consolidates [Reconstruct Uncloud in Rust without adding machinery] and the ten preceding research and product decisions. Where those disagree, this brief states which one wins. Where they left a hole, §12 links the follow-up decision rather than quietly filling it.

**When sources disagree**, authority runs: explicit decisions on the map → stated Uncloud architectural intent → observable product and CLI semantics → documented behaviour → current implementation detail. Implementation detail loses.

---

## 1. The product in one page

Ployz deploys Docker containers across a handful of your own Linux machines and gives you HTTPS, internal DNS, and a CLI, without a control plane.

You run `ployz machine init` against a remote host over SSH. It installs Docker and `ployzd`, creates a cluster, allocates that machine an IPv4 `/24`, reserves a public domain from a hosted DNS service, and deploys Caddy. You run `ployz machine add` for each further host: same provisioning, a WireGuard peering, a new `/24`, another Caddy. Every machine now runs the same daemon, maintains a local eventually convergent replica of cluster observations, and can serve as the CLI's entry point. There is no leader and no manager node.

You then `ployz run IMAGE` or `ployz deploy` from a Compose file. The CLI reads a snapshot of the cluster through one entry machine, computes a finite ordered plan, shows it to you, and executes it by calling target machines directly. Containers get real bridge IPs routed untranslated over the mesh. Caddy picks up new HTTP upstreams from replicated state and issues certificates. Internal DNS resolves `<service>.internal` to healthy container IPs.

Nothing continues after the command returns. There is no controller, no reconciler, no rescheduler. Docker restarts a crashed container on its own machine; that is the entire automatic recovery story. A machine dying does not move its workloads. A machine joining does not gain replicas of a `global` service until you deploy again.

That is not an unfinished product. It is the product. Uncloud's [design note](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/misc/design.md#L99-L122) chose imperative operations because their failures are predictable, and chose availability over agreement because the target cluster is small and low-churn. Ployz keeps that bet.

**Ployz differs from Uncloud in identity, not behaviour.** Binary `ployz`, daemon `ployzd`, env `PLOYZ_*`, config `~/.config/ployz/config.yaml`, socket `/run/ployz/ployz.sock`, bridge network `ployz`, Unix group `ployz`, labels and injected variables under `ployz`. No Uncloud-compatible aliases, no config migration, no wire or storage interop with any Uncloud cluster. ([Choose the preserved, changed, and excluded product surface])

---

## 2. Seven rules that override your instincts

### R1 — Climb the ladder before writing anything

Avoid the subsystem → reuse what is here → Rust std → platform (Linux, Docker, systemd, iptables, WireGuard kernel module) → an already-chosen dependency → a mature external tool → and only then code Ployz owns. The map applies `/ponytail` at full intensity. §9 already ran this ladder for every dependency; the answer is three pieces of owned code and nothing else.

### R2 — Do not add machinery

Forbidden as incidental reconstruction work, at every layer, without a new explicit product decision:

consensus · quorum · leader election · fencing tokens · leases · reservations · distributed locks · cross-machine transactions · general rollback · desired-state reconciliation · background repair loops · rescheduling · relocation · rebalancing · draining · automatic winner selection among duplicates · relays · hole-punch coordination · centralized IPAM · topology controllers · ACLs · security groups · a second overlay · a `doctor`/diagnostics command.

The last one is worth naming: the model deliberately keeps duplicate names, subnets, addresses, keys, and allocations *representable*. Surfacing them is a future effort. ([Reconstruct Uncloud in Rust without adding machinery], Out of scope)

### R3 — Preserve the weakness by outcome, not by transcription

You are not copying Go. You are reproducing what an operator can observe. "Uncloud does it in this order with these goroutines" is not a constraint. "A failed deploy leaves its completed prefix in place and nothing rolls it back" is.

Not frozen: timers, intervals, backoff values, iteration order, map order, error-aggregation mechanics, internal sequencing, which entry wins an ambiguous overwrite, the exact current behaviour of silently omitting partially-replicated rows. ([Freeze architectural invariants and preserved weaknesses])

Frozen: which operations executed, what persisted after a failure, and what did *not* happen.

### R4 — Types expose uncertainty; they never promise

Strong Rust types are the point of this project — [Freeze architectural invariants and preserved weaknesses] permits them and [Define the Rust ubiquitous language and state model] requires several. But a type may only make an existing distinction legible. The moment a type causes a new refusal, a new global check, or a new repair, it has changed the architecture.

Allowed: rejecting the same invalid input earlier and more locally. Example — upstream accepts a `--network` narrower than `/24` at the CLI and only fails later during machine registration; Ployz may reject it at parse time, because it is the same input being rejected for the same reason.

Not allowed: rejecting an input upstream accepts, deduplicating, choosing a canonical winner, or requiring completeness.

### R5 — Do not mirror Go

No Go package layout, type names, control flow, schemas, or wire formats. `~8,600` lines of generated protobuf and its hand-written conversion layer are deleted by §9's choices; do not recreate their shape. Upstream file paths in this brief are evidence to read, not templates to port.

### R6 — Use `CONTEXT.md` vocabulary

`CONTEXT.md` on `main` is the canonical glossary and is binding. Name things from it. Its `_Avoid_` lines are the important part: they list the terms that smuggle in guarantees. If you catch yourself writing `desired state`, `cluster truth`, `authoritative`, `replica identity`, or `reconcile` (in the background-loop sense), stop.

### R7 — A TODO records a ceiling; it is not a work item

Upstream carries 151 authored TODO-style markers. They mark behaviour someone already decided not to build. Carrying one into Ployz is mandatory where it affects behaviour (§10). Implementing one is a product decision you do not have.

### Rust-specific traps

These are the mistakes a competent Rust developer will make *because* they are competent. Each one silently strengthens a guarantee.

| Reflex | What it breaks | Do instead |
|---|---|---|
| `impl Drop` that cleans up a partially-created container / volume / peer | Invents rollback | Leave the artifact. Upstream retains failed containers deliberately, for inspection |
| `?` early-return out of a plan executor | Discards the completed prefix | The executor's error value must *carry* completed prefix, failed operation, and unexecuted suffix |
| `.collect::<Result<Vec<_>, _>>()` over a fan-out | Turns a Partial Result into total failure | Collect successes and per-target failures side by side; both are the result |
| `HashMap<Name, T>` | Silently deduplicates Name Ambiguity | Key by durable ID. Name lookup returns zero, one, or many — and many is a normal answer, not an error |
| A `OnceCell`/singleton allocator for subnets or addresses | Becomes centralized IPAM | Allocation is a locally computed candidate from a stale snapshot, recomputed each time |
| A typestate machine over Machine or Container lifecycle | Machines and containers change outside your process | Plain validated value types plus snapshot enums |
| `#[serde(deny_unknown_fields)]` on a Docker or store boundary | Docker and the store evolve independently | Keep an explicit unknown/raw variant. `Container Runtime Observation` requires one |
| A `tokio::spawn` loop that watches state and "fixes" it | That is a reconciler | Background work is limited to the observer, publisher, projection, and network-maintenance responsibilities in §4 |
| `Arc<Mutex<ClusterState>>` shared across a command | Implies one coherent view | A Deploy captures one snapshot and works from it. The snapshot is advisory and goes stale |
| Generic traits with one implementor | `/ponytail` R1 | One rolling strategy exists. One `DeploymentOperation` enum. `Connector` is the sole permitted single-purpose trait, and only because a WireGuard variant is planned by [Choose technology bindings and owned-code boundaries] |

---

## 3. Domain model

`CONTEXT.md` holds the glossary. This section holds the semantics the glossary compresses.

### Authority is layered, not global

| Rank | Authority | For |
|---|---|---|
| 1 | A Machine's own local state file | Its Machine ID, name, key pair, assigned subnet, Management Address, Advertised Endpoints, Selected Endpoints |
| 2 | Local Docker | Whether a container exists and runs, on that Machine only |
| 3 | Machine-local SQLite (`machine.db`) | The Resolved Service Spec attached to each local container |
| 4 | The local Corrosion replica | A redacted, replicated, possibly stale observation of all Machines and containers |
| 5 | Membership Observation | One Machine's liveness judgement layered on (4) |
| 6 | DNS and Caddy | Asynchronous projections of (4) |

These are not interchangeable. A Live Observation is a direct query to a Machine; a Replicated Observation is a read of the local store. Live does not mean complete, current, or authoritative.

### Service is derived, not stored

There is no services table anywhere. A Service *is* the set of managed Docker containers carrying its Service ID label. Creating the first container creates the Service; removing the last container (including hook containers) removes it. It has no state of its own, no persisted spec, and no lifecycle independent of its containers.

Consequences you must not engineer away:

- Inspecting a Service broadcasts to Machines currently judged up or suspect, collects what answers, and warns about the rest. The answer is legitimately incomplete.
- Each Service Container carries the Resolved Service Spec it was *created with*. During and after a rolling update, one Service can contain several different specs. There is no "current" spec.
- Two concurrent creations can produce two Service IDs with one Service Name. Lookup detects this and demands an ID. It does not prevent it and does not repair it.
- `Service Name` is cluster-global. Compose project names are deliberately **not** prefixed, so two projects with a service named `api` address the same Ployz Service.

Do not model `ServiceState::{Running, Stopped}`. A Service is an arbitrary mixture of running, stopped, failed, unreachable, and differently-specced containers. Model that as an observation with explicit completeness.

### Identity

| Concept | Identity | Selector |
|---|---|---|
| Machine | opaque 32-char lowercase hex, random | Machine Name, ambiguous |
| Service | opaque 32-char lowercase hex, random | Service Name, ambiguous, DNS label ≤63 chars |
| Container | Docker's container ID | generated name `<service>-<4 random chars>`, display only |
| Docker Volume | `(Machine ID, Docker Volume name)` | Service Volume Reference, service-local, distinct |

Keep every identity a distinct newtype. Keep `Service Volume Reference` and `Docker Volume name` distinct — a spec may alias an existing Docker Volume under a different service-local name.

### Closed sets that should be enums

Upstream weakens these to strings and validates at runtime. Encode the validated variants; you delete validation branches for free.

- `ServiceMode::{Replicated { replicas: NonZeroU32 }, Global}` — global with a replica count is currently expressible and meaningless. Note `replicas: 0` is rejected: no scale-to-zero.
- `PortPublication::{Ingress { hostname, load_balancer_port, container_port, http_protocol }, Host { bind, published_port, container_port, transport_protocol }}` — currently independent optional strings.
- `VolumeSource::{Bind, Named, Tmpfs}` — currently a discriminator plus three optional structs. Mutually exclusive.
- `ContainerKind::{ServiceReplica, PreDeployHook}` — currently an enum in one place and label-presence in another.
- `PullPolicy::{Always, Missing, Never}`, `UpdateOrder::{StartFirst, StopFirst}` (optional only in unresolved input, where absence means *derive*).
- `SpecChange::{UpToDate, NeedsUpdate, NeedsRecreate}`.
- Container Runtime Observation: `Created | Running { health } | Paused | Restarting | Exited { code } | Removing | Dead | Unknown { raw }`, with `health: NotConfigured | Starting | Healthy | Unhealthy`.
- Local Machine Phase: `Uninitialized | Joining | Participating | Resetting`. Membership Observation: `Unknown | Up | Suspect | Down`. **Separate types.** Joining folds in store catch-up.

Health derivation, preserved exactly: not running / paused / restarting ⇒ unhealthy; running with no healthcheck ⇒ healthy; running with a healthcheck ⇒ healthy only when Docker says `healthy`. Scheduling treats anything not `Down` as available — suspect and unknown Machines stay eligible.

### States that must remain representable

Not bugs. Direct consequences of quorum-free imperative operation:

a registered Machine that is down/suspect/unknown · a Joining Machine waiting indefinitely on a store version · a Service observation missing one Machine's answer · two Service IDs sharing a name · one Service whose containers carry different specs · a `global` Service missing containers on a newly added Machine · a plan with a completed prefix, a failed operation, and an unexecuted suffix · a failed replacement whose stopped new container remains · a failed stop-first compensation where both old and new are stopped · a `synced` container observation that went stale in a partition · same-named Docker Volumes on several Machines holding different data · a removed Machine whose unreset host still runs containers · duplicate Machine Names, Management Addresses, public keys, or overlapping Machine Subnets surviving storage convergence.

---

## 4. Architecture

### Processes

| Process | Where | What |
|---|---|---|
| `ployz` | operator's laptop (Linux, macOS, Windows-via-WSL) | The whole CLI. Owns deploy planning and execution, Compose loading, secret resolution, image build/push |
| `ployzd` | every cluster Machine (Linux amd64/arm64) | Machine identity, WireGuard, Docker bridge, firewall, machine API, Docker adapter, internal DNS, Caddy config controller, store client |
| Corrosion | pinned container on every Machine | Replicated SQLite. Not linked, not reimplemented |
| Caddy | `global` Ployz Service on every Machine by default | HTTP/HTTPS ingress, ACME, load balancing |
| unregistry | pinned container on every Machine | Receives direct image pushes |

`ployzd` runs under systemd. Reset works by marking `resetting` and exiting; systemd restarts it empty. That is why systemd is a hard dependency and not an abstraction point.

The CLI holds the deployment logic — not the daemon. Do not migrate planning into `ployzd` for tidiness; the daemon has no deployment concept and gaining one would create a control plane.

### Routing

The CLI stores a set of contexts; each context is an ordered list of connections, tried in order until one works. A context is a local view, not cluster state.

Once connected to an entry Machine, a request carries routing metadata: run here, run on one named target, or fan out to a locally resolved set (`*`, names, or IDs). The entry daemon proxies the rest. Everything downstream is entry-relative:

- The entry's replicated view decides who `*` includes. A Machine that has not replicated to the entry is not in the fan-out.
- Name and ID share one lookup namespace. A duplicate name, or a name equal to some Machine ID, overwrites one entry by iteration order. Preserve the ambiguity; do not preserve which one wins ([Define the executable parity contract], tolerance 4).
- A stale row can be selected and then fail to connect.
- Liveness is the entry's judgement. The responder always reports itself `Up`.
- A fan-out returns successes and per-target failures together. That is a Partial Result, not a transaction failure.

You cannot command across a partition boundary. You can fully administer the reachable side. Both sides can return different Machine sets, different liveness, and different Service observations at the same time — simultaneously, correctly.

### Allowed background activity

Background activity is limited to narrow observation, publication, projection, and network-maintenance responsibilities. Adding a loop whose job is to drive observations toward a desired state is adding a reconciler.

- **Machine self-publisher.** Checks its own row at startup, on trigger, and roughly every minute; republishes only when the replicated row is missing or differs. This is why an unreset removed Machine can resurrect its own row after reconnecting.
- **Docker observer.** Watches Docker events plus a ~30s fallback rescan. Upserts redacted observations of local managed containers, deletes rows for containers no longer present locally. Redaction strips environment values before they reach the store.
- **Projection subscribers.** Internal DNS and the Caddy config controller subscribe to replicated container rows and rebuild their outputs.
- **Network maintenance.** WireGuard peer and observer-local endpoint selection update connectivity as described in §5. Corrosion's own replication and membership loops remain inside its pinned container.

These responsibilities do not reschedule, relocate, or repair containers.

### Store semantics

Corrosion is an eventually-consistent SQLite with gossip, CRDT conflict handling, SWIM membership, and anti-entropy. Local writes are synchronous; cross-Machine visibility is not.

The replicated schema holds cluster key-value data, Machine rows, and container rows. Primary keys cover only row identity. **Machine names, Service IDs, and Service Names have indexes, not uniqueness constraints** — the schema does not encode the domain constraints, and that is deliberate. Service ID and name are derived from container labels.

Corrosion can expose a row before all its columns arrive. Upstream omits rows with empty JSON from listings, so a single local read can be structurally incomplete without erroring. Incomplete replicated data must stay tolerable and must never be presented as completeness. The exact silent-omission mechanic is not frozen by [Freeze architectural invariants and preserved weaknesses]; the tolerance is.

Conflict resolution belongs to the pinned Corrosion binary at cell granularity. Do not implement a domain-level merge, conflict object, or winner. Two rows with different IDs and the same name never conflict at the CRDT layer at all — which is exactly how a duplicate survives convergence.

**Join catch-up barrier.** The registering member captures its per-actor max store versions and hands them to the joiner. The joiner starts WireGuard and its machine API, then waits until its local store reaches those versions with no locally known gaps before starting store-dependent components. The wait has no deadline; it logs every five minutes. Preserve the unbounded wait. It proves a version frontier, not global completeness.

---

## 5. Networking and addressing

Equal Linux machines, flat full mesh, no relays.

| Address | Meaning |
|---|---|
| `10.210.0.0/16` (default, configurable) | Cluster IPv4 pool |
| one `/24` from it | Machine Subnet: that Machine's Docker bridge and container range |
| first usable IPv4 in the `/24` (`10.210.X.1`) | Machine Gateway, also the Machine's IPv4 address |
| remaining, Docker-assigned | Container Addresses |
| `fdcc:` + first 14 bytes of the WireGuard public key | Management Address (IPv6, deterministic, no allocator) |
| management `/128` + Machine Subnet `/24` | that peer's WireGuard `AllowedIPs` |

Data plane is IPv4 between containers. Management plane — machine API and Corrosion gossip — is IPv6 over WireGuard. The old design note says the machine IPv4 lives on WireGuard; the frozen code does not. Follow the code.

Every Machine peers with every other visible Machine: `N*(N-1)` peer configs, 25s persistent keepalive, kernel routes for each peer's management address and subnet. Any Machine-table change rebuilds the *entire* peer set and reapplies. Keep the whole-set rebuild until measurement on a supported cluster size proves it fails.

Container traffic crosses the mesh **untranslated**. `ployzd` allows routing from the WireGuard interface to the bridge, inserts a `DOCKER-USER` accept rule, and inserts a `POSTROUTING` return rule ahead of Docker's masquerade so cross-Machine container traffic is not source-NATed. Remote containers see the real source Container Address. Internet egress still uses Docker's NAT. Do not substitute per-container proxies, host ports, or an east-west ingress path.

**Endpoints.** Advertised Endpoints belong to a Machine: auto-discovered from active non-loopback interfaces (skipping Docker, Ployz, and Tailscale interfaces), plus a public IP from `api.ipify.org` → `ipinfo.io/ip` → `ip-api.com` with a 5s timeout, or an explicit override. Selected Endpoint belongs to one observer's relationship with one peer: start with the first advertised candidate, poll the device ~1s, accept a reverse-learned endpoint, treat a fresh selection as unknown for ~15s, treat an established peer with a handshake older than ~275s as down, rotate to the next candidate when down, persist best-effort locally. Timers are not frozen; the state distinctions are.

This is modest NAT traversal, not universal. Keepalives hold a mapping open, WireGuard roaming learns a reverse endpoint, rotation tries known candidates. Two Machines both behind NAT cannot connect. No STUN service, no TURN, no DERP-like relay, no endpoint authority. Say so in the docs; do not fix it.

**Trust.** WireGuard authenticates peers and that is the whole boundary. A peer's entire `/24` is allowed; the firewall accepts WireGuard→bridge traffic; there are no container ACLs. The machine API is reachable from the management range with no additional transport credentials. Local access is authorized by `root` or the `ployz` Unix group. Cluster membership *is* the authorization boundary. No tenants, roles, per-service ACLs, mTLS identity, or policy language.

**Ceilings, all preserved:** 256 `/24`s in the default `/16` · quadratic mesh · whole-peer-set rebuild on every change · Corrosion bootstraps from every peer · iptables/ip6tables only (firewalld unverified) · Linux daemon only · endpoint discovery is heuristic and unvalidated. Do not invent numeric support limits or convergence SLAs the baseline does not have.

---

## 6. Deploy semantics

A Deploy is a bounded command attempt. It has no ID, no persisted state, no owner, no status endpoint, no retry worker. Where upstream comments say "reconcile", read "calculate the finite operations for this invocation" — put that note beside your planner so nobody grows a controller there.

### Shape

1. Normalize and validate the Requested Service Spec.
2. Capture one snapshot: Machines the entry judges available, plus live volume inspection.
3. Ask the rolling strategy for an ordered operation list.
4. Show the plan, confirm, execute in order, stop at the first error.

Service Name and mode cannot change after the first deploy. Service ID is generated by the first plan and reused by every later plan.

For Compose: resolve every service spec, plan missing volumes, build one service plan per service, then execute **all volume operations before all service operations**.

### The operation algebra

The whole vocabulary. One enum, exhaustively matchable — not a dyn-dispatch plugin interface.

create Docker Volume on a Machine · run container (create, start, health-monitor) · stop container · stop and remove container · replace container start-first · replace container stop-first · stop old pre-deploy hook · run and await pre-deploy hook · sequence.

### Placement

**Replicated:** count of containers over currently-available Machines satisfying placement and volume constraints. Eligible Machine order is randomized, then simple round-robin, prioritizing Machines that already hold up-to-date containers. Non-running containers do not count. Run what is missing, replace what mismatches, remove extras.

**Global:** exactly one container on each currently-available eligible Machine, *computed at deploy time*. It is not a standing invariant. A new Machine gets nothing until the user deploys again. Duplicate or stopped containers are repaired only by a user-run deploy.

Placement does **not** consider image platform support, local image presence under `pull_policy: never`, or memory reservations. Preserved gaps.

### Update order

Explicit order wins. Otherwise: conflicting host ports ⇒ stop-first; a single-replica Service with a mounted named Docker Volume ⇒ stop-first; everything else, including multi-replica with volumes, bind mounts, and tmpfs ⇒ start-first. More than one replica is taken as the user asserting concurrent volume access is fine. Keep the heuristic; do not coordinate leases.

### Spec comparison

Compare each container's Resolved Service Spec against the requested one: `UpToDate`, `NeedsUpdate`, `NeedsRecreate`. Most changes recreate. `NeedsUpdate` is detected but **executed as a recreate**, because in-place update is not implemented — carry that TODO. Ingress ports live on container labels, so a port-only change also recreates.

### Failure

Sequence execution stops at the first error and does not undo prior successful operations. If the first of three replacements succeeds and the second fails, the first stays replaced and the third stays untouched.

The only compensation in the system: when a *replacement's* new container fails health monitoring, stop the new container and keep it for inspection; for stop-first only, attempt to restart the old container if it had been running. Errors while stopping the failed new container are ignored, and the restart itself may fail. Then return an error and stop the rest of the plan. That is narrow, local, and it is the ceiling — no general rollback, no resume, no transaction log.

Health monitoring: default ~5s. Without a healthcheck the container must simply stay running for the period. With one it may finish early on healthy and tolerates transient unhealthy inside the period. `--skip-health` disables it. **After** a deploy, an unhealthy container is dropped from Caddy and added back if it recovers — it is never restarted, replaced, or rolled back by Ployz. Docker's `unless-stopped` restart policy is the only automatic maintenance.

Start-first has an accepted gap: Caddy learns the new upstream asynchronously through replicated state, and the deployer stops the old container without waiting for that. A single-replica Service can therefore blink. Preserved, with its TODO. Do not build a Caddy acknowledgement protocol.

### Pre-deploy hooks

Planned only when the Service has a hook *and* the service plan contains at least one run or replace. Runs on the target of the first such operation. Old hook containers are stopped and cleaned first. The hook uses the Service image and most of its container config but disables restart policy, healthcheck, and published ports, and gets `PLOYZ_HOOK_PRE_DEPLOY=true`. Exit 0 continues; non-zero, timeout (default 5 min), or cancellation stops the whole deploy and retains the container. Hook success is **not** recorded, so a retried deploy runs the hook again — document that hooks must be idempotent.

### Direct service commands

`start`, `stop`, `rm` are not state transitions. Each observes the Service's containers now, then operates on them concurrently and joins the errors. Siblings that already succeeded are not undone. `start` excludes old hook containers; `stop` and `rm` include them. Creating a Service can create named volumes before the plan runs, so a later failure leaves those volumes behind.

Every Service Container gets `PLOYZ_MACHINE_ID`.

---

## 7. Projections, storage, secrets

**Internal DNS.** Authoritative for the internal zone. **A records only**, TTL 0, NXDOMAIN when empty, everything else forwarded to the system resolvers (`/etc/resolv.conf`). Returns healthy Service Containers from the replicated store and **does not filter by the hosting Machine's membership state** — a healthy record on a dead Machine stays visible. Preserved, with its TODO. No SRV, no TXT, no AAAA, no caching, no virtual service IPs, no load balancer.

**Caddy.** A `global` Ployz Service using the official `caddy` image, host ports TCP 80, TCP 443, UDP 443, persistent host directories. When no image is given, the greatest stable official `2.x.x` tag is discovered from Docker Hub, falling back to `latest`. Each daemon rebuilds its local Caddy config from healthy replicated container rows — again **without membership filtering**. Config is adapted via Caddy's admin `/adapt`; adaptation is the only validation and cannot prove the config will load. A load failure keeps the last successful config and retries on the next container change. HTTP/HTTPS only: **L4 TCP/UDP ingress through Caddy is not implemented**; host mode is the supported path, and `ployz run` cannot publish L4 ingress at all.

**Managed public DNS.** Ployz continues to use the hosted service at `https://dns.uncloud.run/v1` and treats its Uncloud-branded domains as opaque values. Reserve with `POST /domains`; the daemon (not the CLI process) retains endpoint and bearer token in replicated state, **in plaintext**. Before publishing, it probes each public Caddy Machine's public IP over HTTP for a verification response containing that Machine's ID, and publishes wildcard A and AAAA only for the ones that answer. `release` deletes the local record only — the hosted release call is not implemented. Both the plaintext token and the missing release call are preserved TODOs. Machine removal does not update public DNS. Put this TODO beside the integration boundary, verbatim as required by [Choose the preserved, changed, and excluded product surface]:

```rust
// TODO: Replace dns.uncloud.run and Uncloud-branded domains with
// Ployz-hosted DNS once that infrastructure exists.
```

**Storage.** A Docker Volume is machine-local, full stop. The same name on two Machines is two unrelated volumes with different data. Existing volumes constrain placement. A missing named volume for a replicated Service is created on one eligible Machine; for a global Service it is created independently on every eligible Machine; sharing one volume between a global and a replicated Service is rejected because those rules conflict. Volume scheduling mutates only the command's in-memory snapshot so later service plans know where volumes *will* be — it reserves nothing. Anonymous top-level volume creation is unsupported. No migration, replication, backup, attach fencing, or CSI-like layer.

**Secrets.** Resolved on the CLI machine at deploy time, after image build and before planning, once per reference. The resolved value becomes an environment value and is stored **unencrypted** in the distributed service spec and in Docker container config. File-mounted secrets are unsupported. No encryption at rest, no rotation, no provider control plane.

**Configs.** Read by the CLI, carried inside the service spec, copied per container, deleted with the container. Changes require redeploy. External configs and short syntax are unsupported. No config object store.

---

## 8. Product surface

### Preserved

[Choose the preserved, changed, and excluded product surface] preserves:

The complete documented public surface: remote cluster bootstrap and membership; local contexts with ordered connection failover; service creation and lifecycle; Compose loading with the Ployz extensions; builds and direct image push; machine-local volumes; Caddy; managed DNS; WireGuard inspection; logs; exec; proxy; version; prompts; flags; configuration behaviour; environment precedence.

The command families are: `machine` (`init`, `add`, `rm`, `ls`, `rename`, `update`, `rtt`, `logs`) · `ctx` (`ls`, `show`, `use`, `connection`; bare invocation is an interactive switch) · service operations exposed both at the root and under `service` (`run`, `deploy`, `ls`, `ps`, `inspect`, `scale`, `start`, `stop`, `rm`, `exec`, `logs`) · `build` · `image` (`ls`, `push`) · `volume` (`create`, `ls`, `inspect`, `rm`) · `caddy` (`deploy`, `config`, `logs`) · `dns` (`reserve`, `show`, `release`) · `wg show` · `proxy` · `version`.

The operating envelope: CLI on Linux and macOS, Windows through WSL. Daemon on Linux amd64 and arm64. Ubuntu and Debian tested; other Linux best-effort. Provisioned hosts need key-based SSH as root or a passwordless-sudo user, and systemd.

Global flags and their environment variables (explicit flag beats environment): `--connect`/`PLOYZ_CONNECT`, `--context`/`-c`/`PLOYZ_CONTEXT`, `--ployz-config`/`PLOYZ_CONFIG`. `machine init`'s local `-c` names the *new* context and does not inherit the override. With no config and no `--connect`, an existing `/run/ployz/ployz.sock` is used automatically. Other inputs: `PLOYZ_AUTO_CONFIRM`, `PLOYZ_DAEMON_VERSION`, `PLOYZ_HEALTH_MONITOR_PERIOD`, `PLOYZ_FAILED_CONTAINER_LOGS_TAIL`, `PLOYZ_SSH_CONTROL_PERSIST`, `DEBUG` (`1`/`true`/`yes` only), plus Compose's own `COMPOSE_FILE` and `COMPOSE_DISABLE_ENV_FILE`.

Config file: `~/.config/ployz/config.yaml`, `current_context` plus named contexts, each an ordered connection list. Directories `0700`, file `0600`.

Preserved negative capabilities — these are surface too: no local machine initialization (`machine init` requires a remote target) · no scale to zero · `dns release` is local-only · Compose validation is incomplete and accepts unknown keys · secrets are plaintext in distributed state · no reconciliation, rescheduling, fencing, or broad rollback · `machine add` redeploys Caddy rather than scaling it, with possible brief downtime · `machine rm` refuses to remove the current entry point while other members exist, and `--no-reset` deliberately leaves containers and data.

### Compose

Load via `docker compose config` (§9) and then apply the Ployz layer. The declared support boundary:

**Supported:** `build`, `command`, `entrypoint`, `image`, `init`, `pull_policy`, `stdin_open`, `tty`, `user`; `configs`, `env_file`, `environment`; `cap_add`, `cap_drop`, `devices`, `gpus`, `pid: host`, `privileged`, `sysctls`, `ulimits`; `cpus`, `mem_limit`, `mem_reservation`, `shm_size`; `healthcheck`, `stop_grace_period`; `logging` (default `local`); service `volumes`, named volumes, bind mounts, tmpfs, volume labels, external volumes, `local` and third-party drivers; `deploy.mode` ∈ {`global`,`replicated`}, `deploy.replicas`; file and inline configs.

**Limited:** `depends_on` (ordering yes, `service_completed_successfully` no — use `x-pre_deploy`, because Ployz models long-running independently-owned services, not jobs) · `ports` (host publishing via `mode: host`; HTTP/HTTPS via `x-ports`; ranges rejected) · `secrets` (resolve into env via `secret://name`; no file mounts) · `deploy.resources` (CPU, memory, device reservations only) · `deploy.update_config` (`order` and `monitor` only).

**Unsupported:** `dns`, `dns_search`, service `labels`, `links`, `mem_swappiness`, `memswap_limit`, custom `networks`, `security_opt`, standard service secret mounts, `storage_opt`; `deploy.labels`, Compose `placement`, `restart_policy`, `rollback_config`; external configs and config short syntax; external secrets and providers other than file, environment, or the `exec` driver behind `x-command`.

**"Unsupported" has two observable meanings and both must survive.** Most unsupported service keys, and `service_completed_successfully`, produce a **warning and loading continues**. These fail loading outright: relative or home-relative bind sources, external configs, external secrets, a bad secret driver, using both `ports` and `x-ports` on one service, and published port ranges. Do not infer strictness from the support matrix — the split is the observable behaviour, and its own TODO says detection of common unsupported fields is incomplete. Carry that TODO; do not "complete" it with broad strict validation.

**The six Ployz extensions** (owned code, brand-neutral, names unchanged): `x-context` (project-level), `x-machines`, `x-ports`, `x-caddy`, `x-pre_deploy` (service-level), and `secrets.<name>.x-command`.

**Image tags:** a service with `build` and no tagged `image` gets a Git-aware generated tag (project, service, commit date, 7-char SHA, `.dirty`). An untagged name gets the generated tag; a fully tagged image is left alone. Template functions and Compose interpolation are supported.

**Volume names:** upstream strips the Compose project prefix from volume names. `docker compose config` will emit them prefixed — strip it, or you will create differently-named volumes than the baseline.

### Changed

The following changes come from [Choose the preserved, changed, and excluded product surface], as amended by [Choose technology bindings and owned-code boundaries] and [Define the executable parity contract]:

- Every Ployz-owned identifier renamed. No deprecated Uncloud aliases anywhere.
- Provisioning installs Ployz-owned Rust artifacts, not the Go daemon. (Where from: see §12.)
- The command tree is declarative Clap. Clap-native parsing, diagnostics, help, and completion are approved outcomes. **Do not build compatibility machinery to imitate Cobra.**
- **[Define the executable parity contract] supersedes the "preserve command structure, aliases, hidden exec options, and completion" clause in [Choose the preserved, changed, and excluded product surface].** The Ployz tree may diverge where Ployz prefers a different shape. Every divergence gets one line in the deviation ledger (§11). Everything else in the product-surface decision stands.
- The Docker Compose plugin becomes a client-side prerequisite for `ployz deploy` (the plugin binary only — no running Docker daemon needed). Its scope for other Compose-aware commands is decided by [Define the Docker Compose plugin prerequisite scope].
- `ssh` becomes an explicit client-side prerequisite.
- `ssh_go` and `ssh_cli` config fields are dropped, and with them the `ssh+go://` and `ssh+cli://` schemes. `--connect` accepts `[ssh://]user@host[:port]`, `tcp://host:port`, `unix:///path`.
- `ployz caddy config` prints plain, without syntax highlighting.

### Excluded

[Choose the preserved, changed, and excluded product surface] excludes:

Reading or migrating Uncloud configuration · any Uncloud protocol, storage, cluster, or daemon interop · compatibility aliases for Uncloud names · native Windows and non-Linux daemon targets · the hidden documentation-generator command (the *generated pages* remain a parity oracle; the generator is not product) · a working WireGuard client connector (§9) · any diagnostic/`doctor` command.

No other product change or capability exclusion is approved.

---

## 9. Technology bindings and the owned-code floor

### Run these, do not link or rebuild them

| Component | How |
|---|---|
| **Corrosion** | Version-pinned Docker container (baseline pins `ghcr.io/unlabs-dev/corrosion:2026.6.15`). Already Rust. Ployz owns only an HTTP API client and a unix-socket admin client. The Go `sqlite`/`sqlx`/`squirrel` stack was query *building* against Corrosion's SQL API, never an embedded database — do not port an ORM |
| **Caddy** | Official image, `2.x.y`. Caddyfile→JSON adaptation happens inside the container via the admin API's `/adapt`. Go imported Caddy purely as typed JSON DTOs; Ployz hand-writes serde structs for the subset it emits. No Caddy library exists in Rust and none is needed |
| **unregistry** | `ghcr.io/psviderski/unregistry:0.4.1` — the exact version Go linked — as a pinned container with the containerd socket mounted and the port published. Build/push semantics unchanged, existing firewall rule still applies, same skip-with-warning conditions (Docker not on the containerd image store, or containerd socket undetectable) |
| **Docker**, **WireGuard**, **systemd**, **iptables/ip6tables** | Platform. Not abstraction points |

### Bindings

| Concern | Ployz binding |
|---|---|
| Compose loading | **Shell out to `docker compose config`**, parse the normalized YAML |
| Docker engine | `bollard`, in-process |
| RPC transport | `tonic` transport, serde-encoded domain types in an opaque payload |
| WireGuard (daemon) | `defguard_wireguard_rs` — covers config *and* interface/address management, replacing both `wgctrl` and `netlink` |
| WireGuard (client) | key types only: x25519 parse/format for `ployz wg` |
| SSH transport and provisioning | system `ssh` shell-out only |
| Local `machine.db` | `rusqlite` (`bundled`), sync, behind `spawn_blocking` |
| Corrosion API / admin | `reqwest` with auth and retry layers / `tokio-util` `LengthDelimitedCodec` + `serde_json` |
| DNS server | `hickory-server` + `hickory-resolver` (parses `/etc/resolv.conf` for free) |
| Registry access | `oci-client` and its `Reference` type |
| Terminal | `anstyle`+`anstream` (arrive with clap), `dialoguer::Confirm`, `indicatif`, `crossterm`, `tabled` |
| YAML | `serde_norway` (`serde_yaml` is archived) |
| sd_notify | `sd-notify`, or ~15 lines to `$NOTIFY_SOCKET` |
| Metrics | `prometheus` |
| CDI | ~10 lines of name validation. No crate — only qualified-name validation is used |
| Misc | `ipnet`, `shell-words`, `humansize`, `HashSet`, `sha2`+`oci-spec`, own error enum, `toml`, `semver`, `uuid`, `backon`, serde, build script + clap, std asserts + `pretty_assertions` |

### The owned-code floor — exactly three things

1. **The transparent machine proxy.** `siderolabs/grpc-proxy` has no Rust equivalent. It stays bounded because gRPC framing is a 5-byte length prefix: one-to-one is a byte forward, one-to-many is merging N framed streams with per-Machine identity injection. **Neither needs payload schema knowledge — which is the whole reason the transport stayed gRPC-shaped.** Do not decode domain payloads in the proxy.
2. **The Compose unsupported-feature classifier** (the warn-and-continue vs error-and-stop split above) **and the six extensions.** These are Uncloud's own layer on top of the Compose loader, so shelling out does not remove them.
3. **Caddy config generation**, both the Caddyfile path (for user `x-caddy` snippets) and the JSON path.

Everything else is a binding. If you are about to write a fourth owned subsystem, you have missed a rung.

### Not ported: the client-side userspace WireGuard tunnel

Verified unreachable dead code: `WireGuardConnector` is referenced only inside its own file, the tunnel package is imported only there, and the constructor that would mint the user's key is never called. No observable behaviour, no config surface. Porting it would mean rebuilding `wireguard-go` + netstack on `boringtun` + `smoltcp` for a path nothing calls.

Two obligations follow: a `// TODO:` beside the connector boundary recording that Ployz intends to implement a WireGuard client connector, and **keep `Connector` a trait** so one can slot in later. Do not collapse it into the SSH path.

### Costs you are accepting

- Dropping protobuf removes `grpcurl` as a debugging and parity oracle.
- `rusqlite` bundles SQLite's C source, so the preserved amd64 + arm64 daemon builds need a cross-compiling C toolchain (`cross` or `cargo-zigbuild`), unlike the pure-Go baseline.
- `docker compose config` moves Compose parsing out of process. Verify before building on it: that `x-` keys survive normalization, how `secrets.*.x-command` interacts with Compose's own `exec` secret provider (a value resolved at config time would land in the normalized YAML), and how profiles and `COMPOSE_FILE` behave.

---

## 10. Preserved limitations and the TODO ledger

### The ledger

Create `UPSTREAM_TODOS.md` **before** implementation. It accounts for all 151 authored upstream markers plus the equivalent non-TODO omissions. Each entry: stable key, immutable pinned upstream link, disposition, Rust location, status.

Dispositions:

| Disposition | Meaning |
|---|---|
| **Preserve boundary** | Affects observable semantics. Carry the comment beside the equivalent Rust decision |
| **Carry TODO** | Product-relevant but not architectural. Carry until a later decision resolves it |
| **Resolve by Rust structure** | Concerns Go code shape or a replaced library. Record the disposition; do not reproduce the Go problem |
| **Migration cleanup / not applicable** | Uncloud-version compatibility. No interop goal ⇒ closed with a reason |
| **Reference only** | Experiments, website, build tooling. Ledgered, creates no runtime work |

Rules: every behaviour-affecting boundary gets an adjacent Rust TODO that explains the accepted limitation *and why Uncloud accepts it*, linking its ledger entry. Eliminated Go structure still needs an explicit disposition. **Never create dead Rust code just to host an old comment.** Every implementation PR links its affected ledger entries and states whether it preserves, exposes, or explicitly supersedes the weakness. Review rejects any guarantee-strengthening change absent a later product decision.

Rough shape of the 151: whole files of them are migration cleanup (seven gRPC protocol-version markers, Corrosion pre-v1 migration, uninstall-script legacy handling) or reference-only (`experiment/`, the website theme, the Makefile, CI workarounds). The behavioural core is small; it is listed next.

### The behavioural core — carry these

| Boundary | Preserved limitation |
|---|---|
| Machine add | No announcement, consensus, minority check, or fence. A minority partition may proceed |
| Machine remove | No unschedulable/draining state; placement can race removal. Reset is asynchronous and optimistic. `--no-reset` leaves the host armed and running. Public DNS is not updated. The CLI will not auto-avoid connecting through the target |
| Join | The membership row is written before the target accepts `JoinCluster`, so a failure leaves a ghost member. Catch-up can wait indefinitely |
| Internal DNS | No membership filtering. A-only, TTL 0 |
| Caddy controller | No membership filtering. Adaptation is the only validation. Last-known-good on load failure |
| Start-first replacement | Does not wait for Caddy to observe the new container. Single-replica blink accepted |
| Service identity | Concurrent creation yields several Service IDs under one name. Container creation does not verify the Service Name matches the existing Service ID |
| Store | `sync_status` records local Docker sync, not cluster freshness — a `synced` row can be stale after a crash or partition. A failed Docker list does not mark stored rows outdated |
| Fan-out | Failed Machines in service and volume listings are warned about and omitted rather than returned as typed partial results. **The Partial Result semantics are preserved; [Define the Rust ubiquitous language and state model] requires the typed form, so type it and keep the same accepted outcomes** |
| Placement | Ignores image platform support, `pull_policy: never` image presence, and memory reservations. Pull policies never pull from other cluster Machines |
| Spec diffing | Mutable resource changes are classified but recreate anyway. Ingress ports live on labels, so port-only changes recreate. Unused volume definitions are unresolved |
| Compose | `depends_on` conditions are not fully turned into ordering. Volume scheduling uses unresolved specs. Unsupported-field detection is incomplete. Unknown `x-pre_deploy` attributes are ignored |
| Deploy | The same deploy object can be run more than once. There is no machine filter that preserves containers on excluded Machines |
| Scale | Scaling a Service whose containers disagree on spec just picks one |
| `run` | Cannot publish L4 TCP/UDP ingress |
| Ingress | L4 through Caddy unimplemented |
| Managed DNS | Plaintext token in the store; no service-side release |
| Registry | The embedded registry is reachable from cluster containers, not only Machine gateways |
| Firewall | firewalld behaviour unverified |
| Endpoints | Discovery does not check link-layer interface type or validate reachability |
| Corrosion | Bootstraps from every peer; a partial bootstrap list for large clusters is not implemented. Config file permissions are loose |
| Installer | Does not install the CLI or create an alias on Machines |
| Volumes | Non-local volume driver behaviour is uninvestigated |
| Exec | TTY behaviour mirrors Compose rather than detecting the terminal dynamically |
| SSH | Connection establishment does not fully honour cancellation |
| Logs | Already-opened log streams can live until parent-context cancellation when another stream fails to open |

Explicit non-implementations without TODO syntax, equally binding: local `machine init` errors out · scale to zero is rejected · anonymous volume creation is rejected · Compose port ranges, external secrets, and external configs are rejected · `depends_on: service_completed_successfully` is rejected · secret file mounts are unsupported · the generic scheduler method is unimplemented (the rolling strategy schedules concretely — do not build a general scheduler for it) · the Darwin daemon paths are explicit stubs; Ployz simply has no daemon on Darwin and needs no cross-platform daemon abstraction.

---

## 11. Verification

Parity is proven against **artifacts**. Upstream `uc` is never executed. No Go toolchain in any developer or CI path. No captured goldens — they would freeze help text, error wording, and diagnostics that [Choose the preserved, changed, and excluded product surface] explicitly freed to be Clap-native. ([Define the executable parity contract])

### Oracles

| Source | Used for |
|---|---|
| Upstream `_test.go` corpus | Semantic cases, **re-expressed in Rust**. Nothing is copied as Go |
| Upstream `test/e2e/fixtures/` | Input data, **copied verbatim** |
| Generated CLI reference pages | Command shape |

Fixtures copied as-is, no rename pass — they contain no branded identifier and their extensions are already brand-neutral: `compose-basic.yaml`, `compose-multi-service.yaml`, `compose-placement.yaml`, `compose-placement-comma.yaml`, `compose-placement-nonexistent.yaml`, `compose-volumes.yaml`, `compose-global-volume.yaml`, `compose-configs.yaml`, `compose-predeploy.yaml`, `configs/test-config.conf`, and the `compose-build-basic/` tree.

### Three layers

**Layer 1 — semantic units.** No binary, no Docker, seconds. ~85 upstream cases re-expressed: `pkg/client/compose` (33), `pkg/client/deploy` (21), `pkg/api` (12), `pkg/client`'s Caddy/DNS/log-merging (15), plus the four CLI cases with real semantics (config serialization, connection validation). The remaining CLI cases test Cobra plumbing Clap provides natively and are not ported. **Also in Layer 1: the upstream Caddy config tests** — `jsonconfig_test.go` (378 lines) and `caddyfile_test.go` (1039 lines) are the ready-made oracle for owned-code item 3, and nothing else pins it.

Porting is **not an upfront project**. It is a definition-of-done rule: a subsystem is not complete until its corresponding upstream cases pass. Never write a test for code that does not exist.

**Layer 2 — command shape.** One structural test walks the Clap tree and asserts it against command, alias, flag, default, and environment-annotation data derived from the generated reference pages. No process spawned. It is an **approved-diff** check: matching passes; differing-and-declared passes; differing-and-undeclared fails.

The **deviation ledger** is free-form, one line per deviation stating the change and its reason. No taxonomy, no per-change map approval. Its job is to make structural drift noticed and intentional, not to slow deliberate redesign — human review is the judgement gate, the test only forecloses silence. It is also the record of what the Ployz command tree becomes; there is no separate decision fixing that tree up front.

**Layer 3 — cluster end-to-end.** A Ployz-owned equivalent of upstream's in-Docker cluster harness: a Docker network per test Cluster, N privileged Docker-in-Docker containers each running `ployzd` with WireGuard tooling and a preloaded Corrosion image, **explicitly pre-allocated host ports** (upstream records random assignment as a flake source across container restarts), init on the first Machine, join for the rest, readiness wait with backoff, teardown. Needs a Ployz-owned multiarch test image and its publishing job.

This layer is not satisfiable until the daemon API and crate layout exist. That is accepted deliberately — Layer 3 is the only layer that can execute the negative-parity matrix, and deferring it would leave §10's review gate unenforced meanwhile.

### Six tolerance rules

Backed by a shared ~80-line test-support module (normalizers, set-equality, bounded-eventually) so the tolerant form is the path of least resistance. That is test support, not architecture-lint machinery.

1. **Timing** — assert eventual outcomes with a bounded wait. Never at a fixed instant, never a specific timer/interval/backoff value.
2. **Ordering** — compare unordered collections as sets.
3. **Error aggregation** — assert *which* operations failed and how the Partial Result composes. Never a joined or formatted error string.
4. **Ambiguity** — assert that Name Ambiguity is preserved and representable and that some outcome is reached. Never which entry wins.
5. **Human-facing text** — help, usage, diagnostics, prompts, table layout, colour, progress are never asserted. Clap-native is approved.
6. **Incidental values** — Container IDs, Machine IDs, Service IDs, timestamps, generated container names, dynamic ports are normalized before comparison.

**What stays exact.** Tolerance covers representation and timing, never distributed outcome: which operations a Deploy Plan executed and in what order within its completed prefix; what persisted after a failure, including durable partial effects and the unexecuted suffix; and what did **not** happen — no rescheduling, no relocation, no general rollback, no reconciliation, no repair, no winner selection.

### Negative-parity matrix

One or a few Layer 3 scenarios per invariant family. Not one per ledger entry. **Each asserts the absence of the machinery as strongly as the presence of the outcome.**

1. **Partition-local mutation** — a reachable partition keeps serving local reads, writes, and administration, with no quorum block and no read-your-writes across entry Machines.
2. **Partial Deploy persistence** — a plan failing mid-sequence leaves its completed prefix, reports the failed operation and unexecuted suffix as a Deploy Outcome, and performs no general rollback.
3. **No autonomous repair** — a Service Container that exits or fails health is not rescheduled, relocated, or replaced elsewhere.
4. **Contradictions remain representable** — duplicate Machine Names, Service Names, Machine Subnets, Container Addresses, or keys survive convergence with no canonical winner selected or repaired.
5. **Stale projection behaviour** — membership-blind DNS and Caddy, brief single-replica interruption during asynchronous Caddy propagation, last-known-good Caddy after a load failure.
6. **Machine-local volume data** — a Docker Volume's data is reachable only on its own Machine, and placement anchoring follows it.

### Gate

| Trigger | Runs |
|---|---|
| Every pull request | Layer 1 + Layer 2 (seconds) |
| Merge to `main`, and nightly | Layer 3 |

A failing Layer 3 scenario is retried once; a second failure is a failure, not a flake. Nightly runs keep the harness from rotting between milestones. Every PR links its ledger entries; every command-shape deviation adds its ledger line in the same PR.

---

## 12. Not decided

Do not treat silence here as permission. Raise these; do not resolve them by writing code.

The open questions are first-class children of [Reconstruct Uncloud in Rust without adding machinery]:

- **Workspace and code boundaries:** [Choose the Rust component and crate layout]. This brief fixes two binaries (`ployz`, `ployzd`), three pinned containers, three owned subsystems, deployment logic in the CLI, and the authority layers in §4, but not the crate graph.
- **Artifact distribution:** [Define Ployz artifact distribution and provisioning]. [Choose the preserved, changed, and excluded product surface] requires Ployz-owned artifacts, but the artifact host, names, integrity/signing checks, and installer download contract remain undecided.
- **Packaging and release:** [Define packaging, release, and operational validation]. Packaging formats, cross-compilation, install/upgrade/uninstall behavior, the release pipeline, migration boundaries, and operational validation remain undecided and are constrained by the `rusqlite` bundled-C cost.
- **Wire version skew:** [Define RPC payload compatibility during Ployz version skew]. Do not assume tolerant or strict serde behavior, unknown-variant fallbacks, or version negotiation until that decision closes.
- **Hosted DNS probe:** [Verify the hosted DNS reachability probe contract]. The Ployz-owned rename rule may affect the public Caddy verification path, but whether the hosted service constrains that path is not yet known.
- **Command shape:** Hidden Docker-compatible `exec` flags and shell completion are deviation-ledger choices under [Define the executable parity contract], not requirements of this brief.
- **Compose prerequisite scope:** [Define the Docker Compose plugin prerequisite scope]. Do not infer from the `deploy` decision how `build`, Compose-aware `logs`, `x-context`, or commands without Compose input behave when the plugin is absent.
- **End-to-end coverage:** [Decide Layer 3 upstream end-to-end coverage]. The six negative-parity families are mandatory; the disposition and landing schedule of the roughly 111 upstream subtests is not yet fixed.
- **Replicated sync status:** [Decide replicated container sync-status representation]. [Inventory deliberate omissions, TODOs, and operational boundaries] requires an explicit keep/rename/remove disposition, not an assumed field and filter.
- **Caddy versions:** [Reconcile Caddy runtime selection with tag pinning]. Do not choose between build-time pinning and preserved runtime stable-tag discovery until that conflict is resolved.
- **Future upstream:** [Choose the post-baseline upstream change policy]. The baseline remains frozen unless and until this decision says otherwise.
- **Execution map:** [Draft the implementation tickets and dependency order] follows the decisions above and converts this brief into implementation-sized work.

## 13. Definition of a faithful reconstruction

Ployz is complete when:

- the preserved product workflows operate under Ployz-owned naming on the supported platforms;
- every approved product change and exclusion is reflected explicitly;
- local types express identity, provenance, ambiguity, partial outcomes, and external or unknown states without implying stronger guarantees;
- Corrosion, Docker, WireGuard, Caddy, unregistry, hosted DNS, and the selected Rust bindings meet their fixed boundaries;
- all 151 TODO-style markers and equivalent omissions have ledger dispositions, with behavior-affecting TODOs adjacent to their Rust boundaries;
- the three verification layers and six negative-parity families pass under the approved tolerances;
- implementation review finds no Go-structure mirroring and no unapproved consensus, fencing, reconciliation, transaction, repair, rescheduling, rollback, security, or compatibility machinery;
- every linked question in §12 that blocks a relevant implementation slice has been resolved rather than answered implicitly in code.

---

## 14. Evidence

`CONTEXT.md` on `main` is the binding glossary.

The five research reports are **not on `main`**. They live on unmerged branches and hold the pinned upstream line references behind every claim here. Read them without checking them out:

```
git show origin/research/product-surface:docs/research/product-cli-parity-surface.md
git show origin/research/domain-model:docs/research/domain-and-service-lifecycle.md
git show origin/research/machine-networking:docs/research/machine-networking-and-technology.md
git show origin/research/distributed-state:docs/research/distributed-state-and-partitions.md
git show origin/research/omissions-todos:docs/research/omissions-todos-and-operational-boundaries.md
```

Decisions: [Reconstruct Uncloud in Rust without adding machinery] · [Inventory the product, CLI, and executable parity surface] · [Extract the domain model and service lifecycle semantics] · [Characterize distributed state, partitions, and contradictions] · [Map machine networking and classify technology commitments] · [Inventory deliberate omissions, TODOs, and operational boundaries] · [Choose the preserved, changed, and excluded product surface] · [Define the Rust ubiquitous language and state model] · [Freeze architectural invariants and preserved weaknesses] · [Choose technology bindings and owned-code boundaries] · [Define the executable parity contract] · [Draft and approve the reconstruction brief].

Upstream, for checking a specific claim only — the research already did the extraction, do not redo it:
`https://github.com/psviderski/uncloud/tree/b7e224a1eff98813b1d1a32034d977be24be994e`

---

*Uncloud's simplicity comes from leaving coordination to the user and local recovery to Docker. Most accidental overbuilding will start with an honest attempt to fix one of the states in §3.*

[Reconstruct Uncloud in Rust without adding machinery]: https://github.com/getployz/ployz2/issues/1
[Inventory the product, CLI, and executable parity surface]: https://github.com/getployz/ployz2/issues/2
[Extract the domain model and service lifecycle semantics]: https://github.com/getployz/ployz2/issues/3
[Characterize distributed state, partitions, and contradictions]: https://github.com/getployz/ployz2/issues/4
[Map machine networking and classify technology commitments]: https://github.com/getployz/ployz2/issues/5
[Inventory deliberate omissions, TODOs, and operational boundaries]: https://github.com/getployz/ployz2/issues/6
[Choose the preserved, changed, and excluded product surface]: https://github.com/getployz/ployz2/issues/7
[Define the Rust ubiquitous language and state model]: https://github.com/getployz/ployz2/issues/8
[Freeze architectural invariants and preserved weaknesses]: https://github.com/getployz/ployz2/issues/9
[Choose technology bindings and owned-code boundaries]: https://github.com/getployz/ployz2/issues/10
[Define the executable parity contract]: https://github.com/getployz/ployz2/issues/11
[Draft and approve the reconstruction brief]: https://github.com/getployz/ployz2/issues/12
[Choose the Rust component and crate layout]: https://github.com/getployz/ployz2/issues/13
[Define Ployz artifact distribution and provisioning]: https://github.com/getployz/ployz2/issues/14
[Define RPC payload compatibility during Ployz version skew]: https://github.com/getployz/ployz2/issues/15
[Verify the hosted DNS reachability probe contract]: https://github.com/getployz/ployz2/issues/16
[Define the Docker Compose plugin prerequisite scope]: https://github.com/getployz/ployz2/issues/17
[Decide Layer 3 upstream end-to-end coverage]: https://github.com/getployz/ployz2/issues/18
[Decide replicated container sync-status representation]: https://github.com/getployz/ployz2/issues/19
[Reconcile Caddy runtime selection with tag pinning]: https://github.com/getployz/ployz2/issues/20
[Choose the post-baseline upstream change policy]: https://github.com/getployz/ployz2/issues/21
[Draft the implementation tickets and dependency order]: https://github.com/getployz/ployz2/issues/22
[Define packaging, release, and operational validation]: https://github.com/getployz/ployz2/issues/23
