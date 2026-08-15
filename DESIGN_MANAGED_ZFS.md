# Managed ZFS Volume (proposal)

`machine init` is out of scope. `--from` stays. Compose `data` is a named volume with a quota; ZFS enforces that quota. Pool `--size` as a percentage is the open cut.

## Settled

| Cut | Decision |
|---|---|
| Send/recv | Out of scope. No transfer, no portable volume ID. |
| Snapshots | Out of this cut. |
| Pool | Operator command `ployz zfs pool create`. Not created on first `x-zfs` deploy. `machine init` is out of scope. |
| Backing | `--size` (file-backed sparse vdev) or `--from POOL` (adopt imported zpool). Mutually exclusive. Never auto-pick. |
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
ployz zfs pool create --size 100G [--machine db-1]
ployz zfs pool create --size 80% [--machine db-1]   # if percentage is accepted
ployz zfs pool create --from tank [--machine db-1]
```

`--size` and `--from` are mutually exclusive. `--from` is `zpool get` plus a parent dataset `tank/ployz` — no vdev inventing, no pool picking. A percentage, if accepted, is resolved once at create time to a byte vdev size (not a live fraction). Re-running create against an existing Ployz pool is a conflict, not an ensure. `machine init` does not call this.

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
| Live percentage (quota tracks disk forever) | Resolve once to bytes. ZFS `refquota` is a byte cap. |

## Open questions

❓ **Q1** - **Percentage**: What may be a percentage?

➡️ Pool `--size 80%` only, meaning 80% of **available** space on the backing filesystem, resolved once to a sparse vdev byte size. Volume quota stays `10G`. `--from` ignores `--size`. Volume `x-zfs: 20%` is later sugar.
