# Managed ZFS Volume (proposal)

Default pool is that Machine’s backing FS minus headroom (formula still open: `min(100GiB, 30%)` vs `max`). Sparse vdev. Usage is best-effort. Deploy only checks packing claims.

## Cluster vs Machine

The CLI talks to a Cluster **entry**. The pool lives on a **Machine**. Same split as `uc volume create` vs `uc volume ls`.

| | Cluster (entry / observation) | Machine (the resource) |
|---|---|---|
| Pool | There is none. Do not store “ZFS-enabled.” | `CreateZfsPool` on one target. `--machine` required when more than one Machine exists (prompt if omitted, like `volume create`). |
| Pool / `--size` | There is none at Cluster scope. | Default: backing FS minus headroom, capped by **available**. Sparse vdev (`truncate`, not `fallocate`). `--size` / `--from` override. |
| List | Fan-out Live Observation. Partial Result. | Each row is `{Machine ID, pool, volumes}`. |
| Deploy planning | Walks the observation. Pins like Named volumes. | Allocatable is computed per Machine. Never summed. |
| Named volume `data` + `x-zfs: 10G` | Compose is cluster-entry input. | Dataset is created on the pinned Machine. |

`--machine` was optional in an earlier draft. That made `ployz zfs pool create --size 80%` look like one cluster pool. It is not. Two Machines with `--size 80%` are two pools, two byte sizes.

Fan-out create (`--machines *`) is a later CLI sugar: N machine creates, still N pools. Not this cut.

## Settled

| Cut | Decision |
|---|---|
| Send/recv | Out of scope. No transfer, no portable volume ID. |
| Snapshots | Out of this cut. |
| Pool | Machine command `ployz zfs pool create --machine …`. Not a cluster pool. Not created on first `x-zfs` deploy. `machine init` is out of scope. |
| Backing | Sparse file vdev (`truncate`) or `--from POOL`. No `fallocate`. Usage is best-effort: Docker and ZFS compete for host blocks. |
| Pool size default | Omit `--size`: `pool = available - headroom`. Fail if `pool <= 0`. Headroom formula still open (see scenarios). `--size` / `--from` override. |
| Quota packing | Deploy-time only. `sum(declared quotas on that Machine) + Q ≤ pool_bytes`. No `refquota`. No `refreservation`. Writes are best-effort until the host or pool `ENOSPC`s. |
| Cluster flag | None. ZFS is Live Observation on each Machine (`Ready` / `PoolMissing` / …). Not stored in the replicated store. |
| Privilege | Privileged `ployzd` on ZFS Machines. No helper. No sudo-from-unprivileged. |
| Identity | `{Machine ID, ManagedZfsVolumeName}` |
| Compose | Top-level named volume `data:` plus `x-zfs: 10G`. Service still mounts `data:/path`. |
| Quota | Required packing claim (`x-zfs: 10G`). Planner enforces fit against **that Machine’s other claims**. Not a cluster quota. |
| Not a Docker Volume | Same compose shape as a named volume. Distinct identity, RPC, and snapshot field. Not `VolumeSource::Named`. |
| Consume | Daemon bind-mounts the dataset. Spec stays `VolumeSource::ManagedZfs`. |

## What the user writes

```yaml
services:
  db:
    image: postgres:16
    volumes:
      - data:/var/lib/postgresql/data

volumes:
  data:
    x-zfs: 10G
```

This is a compose named volume (`data`), not a bind. `ployz deploy` does not grow new flags. `x-zfs` cannot sit next to `driver`, `driver_opts`, or `external`.

A Machine without a Machine ZFS Pool is ineligible for `ManagedZfs` placement. That is a Partial Result, not a cluster failure.

## Operator pool command

Deploy does not pick a size. Omit `--size` to use the headroom default.

```
ployz zfs pool create --machine db-1
ployz zfs pool create --machine db-1 --size 100G
ployz zfs pool create --machine db-1 --from tank
```

Same targeting as `ployz volume create`: one Machine. Multi-machine and no `--machine` → prompt.

Default size on the backing FS that will hold the vdev (`/var/lib/ployz/zfs/`). Sparse `truncate`, not `fallocate`. Headroom is a planning number, not reserved host blocks.

```
headroom = min(100GiB, 0.30 × fs_total)   # recommended; max(100GiB, 30%) also shown
pool     = fs_available - headroom
fail if pool <= 0
truncate(vdev, pool)
```

| Disk | A `min(100,30%)` | B `min(100,20%)` | C `min(100,max(8,10%))` | E `<64G→8G` else `min(100,25%)` |
|---|---:|---:|---:|---:|
| 20G | FAIL if 30G min | FAIL | FAIL | FAIL |
| 32G | 22G | 26G | **24G** | 24G |
| 40G | 28G | 32G | **32G** | 32G |
| 64G | 45G | 51G | **56G** | 48G |
| 80G | 56G | 64G | **72G** | 60G |
| 128G | 90G | 102G | **115G** | 96G |
| 256G | 179G | 205G | **230G** | 192G |
| 512G | 412G | 412G | **461G** | 412G |
| 1T+ | 924G / 1.9T / 3.9T | same 100G cap | same 100G cap | same 100G cap |

30G min disk: 20G machines cannot get a pool. Headroom on 32–80G is 6–24G depending on formula (see Q1).

`--from` is `zpool get` plus `tank/ployz` on **that** Machine. Re-running create against an existing Ployz pool on that Machine is a conflict. `machine init` does not call this.

## What happens on one deploy

```
operator: CreateZfsPool on machine db-1   (Machine RPC, not a cluster resource)
compose x-zfs
    → VolumeSource::ManagedZfs
    → ListManagedZfsVolumes             (call, Partial Result)
    → pin like a Named volume
         skip Machines with no pool / no room for Q / no privilege / no tools
    → EnsureManagedZfsVolume            (invoke, always)
    → CreateContainer
         daemon bind-mounts the dataset
```

Always-Ensure for the dataset. Raising the packing claim is a planner check, then Ensure. A claim-only change does not recreate containers.

## Planning (deploy-time fit, commit per Machine)

The Cluster entry **observes**. Each Machine **commits**. Enforcement is: this Deploy’s Q must fit next to the **declared quotas already on that Machine**. Ployz does not police writes at runtime.

```
fan-out observe
    → each Machine: Ready?, pool_bytes, existing {name, declared_quota}

eligible for a new volume with quota Q
    → Ready and sum(declared_quota) + Q ≤ pool_bytes

eligible for an existing name
    → pin to the Machine that already has it
    → raising Q: sum(others) + new_Q ≤ pool_bytes
    → do not move it to a emptier Machine

commit
    → EnsureManagedZfsVolume on the chosen Machine only
```

Among Machines that fit: shuffle then first. One service, two volumes: both claims must fit on that one Machine. Two services that do not share a volume may land on different Machines.

No `refreservation`. No `refquota`. Used-bytes monitoring is later. A noisy volume can fill the host.

Global still means N datasets (Q on each eligible Machine), not one Q split.

## RPC

| RPC | Primitive | Why |
|---|---|---|
| `CreateZfsPool` | `invoke` | Operator only. `--size` or `--from`. Fails if a Ployz pool already exists. |
| `ListManagedZfsVolumes` | `call` | Deploy Snapshot + CLI. Returns capability, pool observation, volumes. |
| `EnsureManagedZfsVolume` | `invoke` | Create the dataset. Persist the declared quota as a user property for the next Deploy’s packing math. |
| `RemoveManagedZfsVolume` | `invoke` | Destroy the dataset. Not a Deploy operation. Not compensation. |

Capabilities: `ployz.zfs.pool.create.v1`, `ployz.zfs.list.v1`, `ployz.zfs.ensure.v1`, `ployz.zfs.remove.v1`.

No send/recv RPCs. No snapshot RPCs. No `CreateVolume` reuse. A Managed ZFS Volume must never appear in `VolumeList`.

`Inspect` of a volume is a client filter of `List` on one Machine.

Observe reports: `Ready | ToolsMissing | PrivilegeMissing | PoolMissing | NestedZfsBlocked`. Nested ZFS: creating a file vdev on an existing ZFS dataset is refused.

## Domain

| | Docker Volume | Managed ZFS Volume |
|---|---|---|
| Identity | `{Machine ID, DockerVolumeName}` | `{Machine ID, ManagedZfsVolumeName}` |
| `VolumeSource` | `Named` | `ManagedZfs` |
| Snapshot field | `volumes` | `managed_zfs` |
| Deploy op | `CreateVolume` | `EnsureManagedZfsVolume` |
| Container mount | Docker `volume` | Docker `bind` of a daemon-owned path |
| Quota | none | packing claim, deploy-time only |
| Backing | Docker | Machine ZFS Pool (operator-created) |

The Resolved Service Spec keeps `VolumeSource::ManagedZfs`. Do not rewrite it to `Bind`.

## Privilege

ZFS needs `/dev/zfs`. `ployzd` on a ZFS Machine runs privileged enough to do that. `observe` still reports `PrivilegeMissing` if it cannot (wrong install). Planner excludes that Machine.

## How we would build it

1. `VolumeSource::ManagedZfs` + `x-zfs` parse + compose tests. No daemon.
2. Four catalog rows + in-memory adapter. Planner pin + allocatable check + always-Ensure + skip `PoolMissing`.
3. Daemon `LocalManagedZfs` + `FakeZfsPlane` (create pool, refuse nested, `PoolMissing`).
4. `CreateContainer` bind arm. One Linux test gated on `/usr/sbin/zfs`.
5. CLI `ployz zfs pool create --machine` with headroom default. No `machine init` flag.

## Rejected

| Shape | Why |
|---|---|
| `driver: zfs` / Docker plugin | Collapses Managed ZFS Volume into Docker Volume. |
| Reuse `VolumeSource::Bind` | Convert does not know the Machine path. Planner ignores Bind. |
| Rewrite spec to Bind after Ensure | Recreate loop on the next Deploy. |
| Cluster-scoped volume ID | A Cluster is not authoritative. Send/recv is out of scope. |
| Hidden pool on first Ensure | Operator runs `pool create`. Default size is the headroom formula, not “whatever fits.” |
| Sparse vdev | ~~rejected~~. Usage is best-effort. Docker and ZFS share host blocks. |
| `fallocate` | Operator said no allocate. Headroom is not held. |
| Cluster-wide quota | Fit is `sum(claims on that Machine)`. Never summed across Machines. |
| Runtime write policing as the product | Deploy refuses a claim that does not fit. Monitoring later for used-bytes. |
| Send/recv / snapshot RPCs | Out of scope. |
| Unprivileged daemon + helper | Second privilege story. Privileged `ployzd` instead. |
| ZFS `quota` (includes snapshots) | One snapshot would steal the app's write budget. Use `refquota`. |
| `machine init` creates the pool | Out of scope. Operator runs `ployz zfs pool create`. |
| Auto-pick an imported zpool | Operator passes `--from`. Guessing is the hard part; naming is not. |
| Live percentage (pool or quota tracks disk forever) | Resolve `--size 80%` once to bytes. ZFS caps are byte sizes. |
| Volume `x-zfs: 20%` | Out of this cut. Volume quota is `10G`. |
| Stored “ZFS-enabled cluster” | Cluster truth. Observe each Machine. |
| Cluster zpool / optional `--machine` | Same bug as treating `volume create` as cluster-wide. Create targets one Machine. |
| Fan-out `pool create` on every Machine | Later sugar. Still N machine pools, not one cluster pool. |
| Default `--overcommit 120%` | Replaced: packing is 100% of that Machine’s pool. |
| `refreservation` in v1 | 10G would be a held slice. User: packing claim now, monitor used later. |
| Move an existing volume to a emptier Machine | Send/recv is out. Pin stays. |
| Cluster bin-pack (most-free / knapsack) | Shuffle then first fit, like today’s volume planner. |

## Uncloud (what people actually said)

Full notes: `evidence/uncloud-zfs-wants.md`.

Uncloud has **no ZFS product**. One author comment ([#242](https://github.com/psviderski/uncloud/issues/242#issuecomment-3771471639), 2026-01-20): still-local volumes with snapshots, backups, restore-elsewhere; ZFS named as an example next to device mapper. Distributed storage (Gluster/Ceph) rejected. Users asked for NFS `driver_opts`, postgres backups, and not losing a volume when a machine is down. Nobody asked Uncloud for quotas, `x-zfs`, or a pool CLI.

This Ployz design is original. Snapshots/send/recv matching the author’s “recover” story stay out of this cut.

## Open questions

❓ **Q1** - **Headroom + 30G min**: A `min(100,30%)` / B `min(100,20%)` / C `min(100,max(8,10%))` / E piecewise. All refuse under 30G.

➡️ **C** plus 30G min. Nicest to small: ~8G for Docker, rest is pool. 20G machines cannot host a file-backed pool honestly.

❓ **Q2** - Two compose projects both named `data`: one dataset or two?

➡️ Two. `{Machine ID, project_key}`.

❓ **Q3** - Who imports the file-backed pool after reboot?

➡️ `ployzd` on start.
