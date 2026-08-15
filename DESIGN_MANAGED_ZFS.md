# Managed ZFS Volume (proposal)

Not accepted. Do not add these terms to `CONTEXT.md` until the three questions at the end are settled.

A Managed ZFS Volume is a machine-local ZFS dataset with a required quota. It is not a Docker Volume, not a Bind Mount, and not a cluster volume.

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

`ployz deploy` does not grow new flags. `x-zfs` cannot sit next to `driver`, `driver_opts`, or `external`.

Object form is optional later:

```yaml
volumes:
  data:
    x-zfs:
      quota: 50G
      compression: lz4
```

`quota` in YAML means ZFS `refquota` (live data only). Snapshots must not steal the app's write budget.

## What happens on one deploy

```
compose x-zfs
    → VolumeSource::ManagedZfs
    → observe every Machine          (call, Partial Result)
    → plan: pin like a Named volume
    → EnsureManagedZfsVolume         (invoke, always)
    → CreateContainer
         daemon bind-mounts the dataset
```

Always-Ensure. The planner does not skip create because a dataset already exists. Quota converges in place. A quota-only change does not recreate containers.

## RPC (three rows)

| RPC | Primitive | Why |
|---|---|---|
| `ListManagedZfsVolumes` | `call` | Deploy Snapshot + CLI list. Returns capability, backing, volumes (id, quota, used, mountpoint). |
| `EnsureManagedZfsVolume` | `invoke` | Create or set `refquota`. Idempotent. |
| `RemoveManagedZfsVolume` | `invoke` | Destroy the dataset. Not a Deploy operation. Not compensation. |

Capabilities: `ployz.zfs.list.v1`, `ployz.zfs.ensure.v1`, `ployz.zfs.remove.v1`.

Do not add pool RPCs. Do not reuse `CreateVolume`. A Managed ZFS Volume must never appear in `VolumeList`.

`Inspect` is a client filter of `List` on one Machine. No fourth row.

## Domain

| | Docker Volume | Managed ZFS Volume |
|---|---|---|
| Identity | `{Machine ID, DockerVolumeName}` | `{Machine ID, ManagedZfsVolumeName}` |
| `VolumeSource` | `Named` | `ManagedZfs` |
| Snapshot field | `volumes` | `managed_zfs` |
| Deploy op | `CreateVolume` | `EnsureManagedZfsVolume` |
| Container mount | Docker `volume` | Docker `bind` of a daemon-owned path |
| Quota | none | required `refquota` |

The Resolved Service Spec keeps `VolumeSource::ManagedZfs`. The daemon looks up name → mountpoint at `CreateContainer`. Do not rewrite the spec to `Bind` — next Deploy would see compose `ManagedZfs` vs live `Bind` and recreate forever.

`Service Volume Reference` (`data`) is still only the name inside one Service spec.

## Pool (hidden, failures visible)

Not a dedicated disk. One of:

1. An imported pool that already has `…/ployz` → use it.
2. Exactly one writable imported pool → create `pool/ployz/vol/<name>`.
3. Several imported pools, none already `ployz` → `AmbiguousPool`. Do not guess.
4. Zero imported pools → one file-backed pool (`ployz`) on a sparse vdev under `/var/lib/ployz/zfs/`.
5. Target dir is already ZFS, or inventory failed → **stop**. Never fall through to a file vdev on ZFS. That is nested ZFS.

Callers must see: `Ready | ToolsMissing | PrivilegeMissing | NestedZfsBlocked | AmbiguousPool`. A Machine that is not `Ready` is ineligible. That is a Partial Result, not a cluster failure.

Overcommit is allowed. Volume `refquota` is not pool size. File-backed ceiling is a daemon default (100 GiB sparse) until someone asks for a pool-size knob.

## Privilege

ZFS needs `/dev/zfs` (typically root or a targeted capability). The daemon does not sudo. `observe` reports `PrivilegeMissing` with the verb that failed. Planner excludes that Machine.

## How we would build it

1. `VolumeSource::ManagedZfs` + `x-zfs` parse + compose tests. No daemon yet.
2. Three catalog rows + in-memory adapter. Planner pin + always-Ensure. Deploy tests with the fake.
3. Daemon `LocalManagedZfs` + `FakeZfsPlane` (policy: existing pool vs file vdev vs refuse nested).
4. `CreateContainer` bind arm. One Linux integration test gated on `/usr/sbin/zfs`.
5. Later, not this cut: CLI `ployz storage`, snapshots, send/recv migrate.

v1 is compose + ensure + quota + pin. Snapshots and machine-to-machine transfer are the north star, not the first interface.

## Rejected

| Shape | Why |
|---|---|
| `driver: zfs` / Docker plugin | Collapses Managed ZFS Volume into Docker Volume. |
| Reuse `VolumeSource::Bind` | Convert does not know the Machine path. Planner ignores Bind → wrong Machine. |
| Rewrite spec to Bind after Ensure | Recreate loop on the next Deploy. |
| Cluster-scoped volume ID | A Cluster is not authoritative. Transfer later copies one machine-local volume to another. |
| Pool / snapshot / send RPCs in v1 | Shallow `zfs(8)` wrapper. Grow the interface when a caller exists. |
| ZFS `quota` (includes snapshots) | One snapshot steals the app's write budget. Use `refquota`. |

## Open questions

Answer these before `CONTEXT.md` or an ADR.

1. **v1 scope** — compose+ensure+quota only, or snapshots+transfer in the first cut?
2. **Pool size** — hidden 100 GiB sparse default, or an operator-visible knob?
3. **Daemon privilege** — require a privileged `ployzd` on ZFS Machines, or a small helper binary?
