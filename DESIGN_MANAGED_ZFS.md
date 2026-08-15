# Managed ZFS Volume (proposal)

No “ZFS-enabled cluster” flag. Overcommit is a per-pool knob. Planning is per-Machine allocatable, same pin as a Named volume. Default overcommit is the open cut.

## Settled

| Cut | Decision |
|---|---|
| Send/recv | Out of scope. No transfer, no portable volume ID. |
| Snapshots | Out of this cut. |
| Pool | Operator command `ployz zfs pool create`. Not created on first `x-zfs` deploy. `machine init` is out of scope. |
| Backing | `--size` (file-backed sparse vdev) or `--from POOL` (adopt imported zpool). Mutually exclusive. Never auto-pick. |
| Pool size | `--size 100G` or `--size 80%`. `%` is of **available** space on the backing filesystem, resolved once to a sparse vdev byte size. `--from` has no `--size`. Volume quota is not a percentage. |
| Overcommit | Pool property, not a cluster property. `allocatable = pool_bytes × ratio`. Planner drops a Machine when a new or raised `refquota` would exceed allocatable. ZFS still returns `ENOSPC` if used hits the pool. |
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
ployz zfs pool create --size 100G [--overcommit 1] [--machine db-1]
ployz zfs pool create --size 80% [--overcommit 2] [--machine db-1]
ployz zfs pool create --from tank [--overcommit 1] [--machine db-1]
```

`--size` and `--from` are mutually exclusive. `--from` is `zpool get` plus a parent dataset `tank/ployz` — no vdev inventing, no pool picking. `--size 80%` is resolved once at create time (`statvfs` available × 0.8 → sparse vdev bytes). It is not a live fraction. Re-running create against an existing Ployz pool is a conflict, not an ensure. `machine init` does not call this.

Volume Ensure then creates `pool/ployz/vol/<name>` with `refquota`.

## What happens on one deploy

```
operator: CreateZfsPool                 (once per Machine, not Deploy)
compose x-zfs
    → VolumeSource::ManagedZfs
    → ListManagedZfsVolumes             (call, Partial Result)
    → pin like a Named volume
         skip Machines with no pool / no privilege / no tools
    → EnsureManagedZfsVolume            (invoke, always)
    → CreateContainer
         daemon bind-mounts the dataset
```

Always-Ensure for the dataset. Quota converges in place. A quota-only change does not recreate containers.

## Planning (quotas are machine-local)

There is no cluster quota. Each Machine has its own pool, size, overcommit, and datasets. `ListManagedZfsVolumes` is fan-out Live Observation. Failures are a Partial Result; those Machines are omitted.

```
observe each Machine
    → capability, pool_bytes, overcommit, existing {name, refquota}
    → allocatable = pool_bytes × overcommit
    → used_alloc = sum(existing refquotas)

eligible for a volume with quota Q
    → Ready
    → if name already exists here: pin here
         Ensure may raise Q; need (used_alloc - old_Q + new_Q) ≤ allocatable
    → if name is new: (used_alloc + Q) ≤ allocatable
    → else drop this Machine
```

Same pin rules as Named Docker volumes, on `managed_zfs` not `volumes`:

| Mode | What happens |
|---|---|
| Replicated + volume missing | Pick one eligible Machine, Ensure once, pin every replica there. |
| Replicated + volume exists | Pin to that Machine. Do not create a second `data` on another Machine. |
| Global | Ensure `data` at quota Q on **each** eligible Machine. That is N datasets, N×Q allocated, not one Q split across the Cluster. |
| Shared `data` across services | One dataset, one Q. Intersection of each service’s eligible set. Mixed global+replicated is rejected (`MixedVolumeModes`). |

`x-machines` still intersects. A Machine with no pool is ineligible, not a cluster-wide failure.

Do not sum quotas across Machines. Do not persist “this Cluster is ZFS-enabled.” `DescribeContract` on a daemon that can talk to ZFS advertises the ZFS capabilities; that is per-Machine, same as Docker.

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
2. Four catalog rows + in-memory adapter. Planner pin + always-Ensure + skip `PoolMissing`.
3. Daemon `LocalManagedZfs` + `FakeZfsPlane` (create pool, refuse nested, `PoolMissing`).
4. `CreateContainer` bind arm. One Linux test gated on `/usr/sbin/zfs`.
5. CLI `ployz zfs pool create`. No `machine init` flag.

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

## Open questions

❓ **Q1** - **Overcommit default**: If the operator omits `--overcommit`, what is the ratio?

➡️ `1` — sum of volume `refquota`s on that Machine may not exceed pool bytes. `--overcommit 2` is the explicit “lie about capacity” knob. Unlimited (ZFS-native, planner never checks allocation) is a later escape, not the default.
