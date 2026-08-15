# Managed ZFS Volume (proposal)

Quota packing is 100% per Machine. The Cluster entry observes who has room, then commits on one Machine. No reservation. Monitoring is later.

## Cluster vs Machine

The CLI talks to a Cluster **entry**. The pool lives on a **Machine**. Same split as `uc volume create` vs `uc volume ls`.

| | Cluster (entry / observation) | Machine (the resource) |
|---|---|---|
| Pool | There is none. Do not store “ZFS-enabled.” | `CreateZfsPool` on one target. `--machine` required when more than one Machine exists (prompt if omitted, like `volume create`). |
| Overcommit / `--size 80%` | Not a cluster policy. `--size 80%` is **that** Machine’s pool file. Quota packing is 100% of **that** pool. Not 100% of the host disk. |
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
| Backing | `--size` (file-backed sparse vdev) or `--from POOL` (adopt imported zpool). Mutually exclusive. Never auto-pick. |
| Pool size | `--size 100G` or `--size 80%`. `%` is of **available** space on the backing filesystem, resolved once to a sparse vdev byte size. `--from` has no `--size`. Volume quota is not a percentage. |
| Overcommit | Default **100%**. `allocatable = pool_bytes`. Planner observes every Machine, drops any that cannot fit Q, commits `Ensure` on one Machine that can. No `refreservation`. Used-bytes / noisy-neighbor is a later monitoring module. `--overcommit` remains an override, not the default story. |
| Not host 100% | `--size 100%` of the backing FS would leave nothing for Docker images, logs, or the OS. Pool size and quota packing are different 100%s. |
| Cluster flag | None. ZFS is Live Observation on each Machine (`Ready` / `PoolMissing` / …). Not stored in the replicated store. |
| Privilege | Privileged `ployzd` on ZFS Machines. No helper. No sudo-from-unprivileged. |
| Identity | `{Machine ID, ManagedZfsVolumeName}` |
| Compose | Top-level named volume `data:` plus `x-zfs: 10G`. Service still mounts `data:/path`. |
| Quota | Required. YAML `quota` / `10G` is ZFS `refquota`. The kernel returns `ENOSPC`. Ployz does not police writes. |
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

Deploy does not pick a size and does not create a sparse vdev.

```
ployz zfs pool create --machine db-1 --size 100G
ployz zfs pool create --machine db-1 --size 80%
ployz zfs pool create --machine db-1 --from tank
```

Same targeting as `ployz volume create`: one Machine. Multi-machine and no `--machine` → prompt. There is no cluster-wide create. Omit `--overcommit` → **100%**.

`--size` and `--from` are mutually exclusive. `--from` is `zpool get` plus a parent dataset `tank/ployz` on **that** Machine. `--size 80%` is resolved once on **that** Machine (`statvfs` available × 0.8 → sparse vdev bytes). Re-running create against an existing Ployz pool on that Machine is a conflict. `machine init` does not call this.

Volume Ensure then creates `pool/ployz/vol/<name>` with `refquota`.

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

Always-Ensure for the dataset. Quota converges in place. A quota-only change does not recreate containers.

## Planning (100% packing, commit per Machine)

The Cluster entry **observes**. Each Machine **commits**. There is no cluster quota and no cluster pool.

```
fan-out observe
    → each Machine: Ready?, pool_bytes, existing {name, refquota}
    → allocatable = pool_bytes          # 100%. No 120%.
    → used_alloc = sum(existing refquotas)

eligible for a new volume with quota Q
    → Ready and (used_alloc + Q) ≤ allocatable

eligible for an existing name
    → pin to the Machine that already has it
    → raising Q: (used_alloc - old_Q + new_Q) ≤ allocatable
    → do not move it to a emptier Machine (send/recv is out)

commit
    → EnsureManagedZfsVolume on the chosen Machine only
```

Among Machines that fit, keep today’s shuffle then first. Do not bin-pack “most free space.” A missing replicated volume still lands on one Machine; all replicas pin there.

One service with two Managed ZFS Volumes: **both** quotas must fit on that one Machine. Two services that do not share a volume may land on different Machines.

No `refreservation`. 10G is a cap, not a held slice. A noisy volume can fill the pool; ZFS `ENOSPC`s. Used-bytes monitoring is a later module, not this cut.

`x-machines` still intersects. A Machine with no pool or no room is ineligible, not a cluster-wide failure.

Do not persist “this Cluster is ZFS-enabled.”

Global still means N datasets (Q on each eligible Machine), not one Q split. Shared `data` across services is one dataset, one Q, intersection of eligible sets.

## RPC

| RPC | Primitive | Why |
|---|---|---|
| `CreateZfsPool` | `invoke` | Operator only. `--size` or `--from`. Fails if a Ployz pool already exists. |
| `ListManagedZfsVolumes` | `call` | Deploy Snapshot + CLI. Returns capability, pool observation, volumes. |
| `EnsureManagedZfsVolume` | `invoke` | Create or set `refquota`. Fails with `PoolMissing` if the operator never created a pool. |
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
| Quota | none | required `refquota` |
| Backing | Docker | Machine ZFS Pool (operator-created) |

The Resolved Service Spec keeps `VolumeSource::ManagedZfs`. Do not rewrite it to `Bind`.

## Privilege

ZFS needs `/dev/zfs`. `ployzd` on a ZFS Machine runs privileged enough to do that. `observe` still reports `PrivilegeMissing` if it cannot (wrong install). Planner excludes that Machine.

## How we would build it

1. `VolumeSource::ManagedZfs` + `x-zfs` parse + compose tests. No daemon.
2. Four catalog rows + in-memory adapter. Planner pin + allocatable check + always-Ensure + skip `PoolMissing`.
3. Daemon `LocalManagedZfs` + `FakeZfsPlane` (create pool, refuse nested, `PoolMissing`).
4. `CreateContainer` bind arm. One Linux test gated on `/usr/sbin/zfs`.
5. CLI `ployz zfs pool create --machine`. Same targeting as `volume create`. No cluster pool. No `machine init` flag.

## Rejected

| Shape | Why |
|---|---|
| `driver: zfs` / Docker plugin | Collapses Managed ZFS Volume into Docker Volume. |
| Reuse `VolumeSource::Bind` | Convert does not know the Machine path. Planner ignores Bind. |
| Rewrite spec to Bind after Ensure | Recreate loop on the next Deploy. |
| Cluster-scoped volume ID | A Cluster is not authoritative. Send/recv is out of scope. |
| Hidden pool on first Ensure | Operator owns size and backing. Deploy must not invent a 100 GiB file. |
| Send/recv / snapshot RPCs | Out of scope. |
| Unprivileged daemon + helper | Second privilege story. Privileged `ployzd` instead. |
| ZFS `quota` (includes snapshots) | One snapshot would steal the app's write budget. Use `refquota`. |
| `machine init` creates the pool | Out of scope. Operator runs `ployz zfs pool create`. |
| Auto-pick an imported zpool | Operator passes `--from`. Guessing is the hard part; naming is not. |
| Live percentage (pool or quota tracks disk forever) | Resolve `--size 80%` once to bytes. ZFS caps are byte sizes. |
| Volume `x-zfs: 20%` | Out of this cut. Volume quota is `10G`. |
| Stored “ZFS-enabled cluster” | Cluster truth. Observe each Machine. |
| Cluster-wide quota | A Managed ZFS Volume is machine-local. Global means N datasets. |
| Cluster zpool / optional `--machine` | Same bug as treating `volume create` as cluster-wide. Create targets one Machine. |
| Fan-out `pool create` on every Machine | Later sugar. Still N machine pools, not one cluster pool. |
| Default `--overcommit 120%` | Replaced: packing is 100% of that Machine’s pool. |
| `--size 100%` of the host disk as default | Starves Docker/OS. Pool size is operator `--size`. Quota packing is 100% of the pool, not of the disk. |
| `refreservation` in v1 | 10G would be a held slice. User: cap now, monitor used later. |
| Move an existing volume to a emptier Machine | Send/recv is out. Pin stays. |
| Cluster bin-pack (most-free / knapsack) | Shuffle then first fit, like today’s volume planner. |

## Uncloud (what people actually said)

Full notes: `evidence/uncloud-zfs-wants.md`.

Uncloud has **no ZFS product**. One author comment ([#242](https://github.com/psviderski/uncloud/issues/242#issuecomment-3771471639), 2026-01-20): still-local volumes with snapshots, backups, restore-elsewhere; ZFS named as an example next to device mapper. Distributed storage (Gluster/Ceph) rejected. Users asked for NFS `driver_opts`, postgres backups, and not losing a volume when a machine is down. Nobody asked Uncloud for quotas, `x-zfs`, or a pool CLI.

This Ployz design is original. Snapshots/send/recv matching the author’s “recover” story stay out of this cut.
