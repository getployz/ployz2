# Managed ZFS storage for Ployz

Design only. No implementation.

## The shape, in five lines

1. A human writes `data:` with `x-zfs: 10G` in compose, deploys, and gets a ZFS dataset with `refquota=10G`.
2. That dataset is handed to Docker as an ordinary named volume bound to its mountpoint. Everything that already works — placement pinning, `ployz volume ls`, `docker inspect` — keeps working untouched.
3. The pool is machine-local, named `ployz`, and lives on a preallocated file on whatever filesystem the Machine already has. No dedicated disk.
4. Docker's data-root moves onto the same pool with its own quota. So *everything that grows* is inside one pool, and nothing can eat the host filesystem.
5. Quotas are enforced at runtime by ZFS, never by deploy-time packing arithmetic. Sentry filling its quota breaks Sentry.

Domain term: **Managed ZFS Volume**, already reserved in `CONTEXT.md` as distinct from a Docker Volume. **Machine Pool** is the machine-local zpool. Neither is Cluster state.

## Compose

```yaml
services:
  web:
    image: sentry:latest
    volumes:
      - data:/var/lib/sentry

volumes:
  data:
    x-zfs: 10G
```

Long form, for when `quota` needs company later:

```yaml
volumes:
  data:
    x-zfs:
      quota: 10G
```

Scalar-or-object mirrors `x-caddy` exactly (`RawCaddy::String | Object`). Unknown keys inside the object are an error, not a shrug — `x-pre_deploy`'s "unknown attributes ignored" TODO is a wart, not a pattern.

Rules:

- `x-zfs` is only valid on a top-level volume, never on a service mount. Storage is a property of the volume, not of who mounts it.
- A volume without `x-zfs` stays a plain Docker Volume. It is still bounded, because it lives inside the Docker dataset's quota (see below). Deploy prints one warning line per unannotated volume when the target Machine has a pool.
- No `driver: zfs`, ever. See "What I refuse".

## Operator CLI

Same targeting as `volume create --machine`: one Machine per mutation, fan-out for reads, Partial Result on fan-out.

```
ployz storage pool create --machine m1 [--size 100G] [--from tank] [--file PATH] [--adopt-docker]
ployz storage pool grow   --machine m1 --size 200G
ployz storage ls          [--machine m1 ...]
ployz storage quota set   data=20G --machine m1
ployz storage rm          data --machine m1 [--yes]
```

`ployz storage ls`:

```
MACHINE	POOL	BACKING	SIZE	USED	FREE	COMMITTED
m1	ployz	file:/var/lib/ployz-zfs/pool.img	56G	15G	41G	50G
m2	tank	adopted	1.8T	400G	1.4T	120G

MACHINE	VOLUME	QUOTA	USED
m1	data	10G	2.3G
m1	sentry-data	40G	38G
m1	<docker>	22G	12G
```

`COMMITTED` above `SIZE` is legal and printed, not blocked. That is thin provisioning working as intended.

`ployz volume ls` output does not change. Managed ZFS Volumes appear there as the Docker Volumes they are.

## RPC

Four catalog rows. Requests live in `ployz-core/src/rpc/zfs.rs` next to `rpc/docker.rs`.

```
EnsureZfsPool:      (ensure_zfs_pool,   "EnsureZfsPool",     EnsureZfsPoolRequest,     "ensure_zfs_pool",     ZfsPoolEnsured)
EnsureZfsVolume:    (ensure_zfs_volume, "EnsureZfsVolume",   EnsureZfsVolumeRequest,   "ensure_zfs_volume",   ZfsVolumeEnsured)
InspectZfsStorage:  (inspect_zfs_storage,"InspectZfsStorage",InspectZfsStorageRequest, "inspect_zfs_storage", MachineStorage)
RemoveZfsVolume:    (remove_zfs_volume, "RemoveZfsVolume",   RemoveZfsVolumeRequest,   "remove_zfs_volume",   ZfsVolumeRemoved)
```

Capabilities, advertised only when the daemon finds usable ZFS userland — gated the way volume capabilities are gated on `self.containers.is_some()`:

```
ployz.zfs.pool.ensure.v1
ployz.zfs.volume.ensure.v1
ployz.zfs.inspect.v1
ployz.zfs.volume.remove.v1
```

`call` vs `invoke`:

| RPC | Style | Why |
| --- | --- | --- |
| `InspectZfsStorage` | `call`, fanned out per Machine into a `PartialResult` like `Client::list_volumes` | Read. Retryable. Some Machines will be down; that is a Partial Result, not a failure. |
| `EnsureZfsPool` | `invoke` with `MachineSelector` + `TARGET_RPC_TIMEOUT` | One-target mutation, exactly like `CreateVolume`. Not retried behind the operator's back. |
| `EnsureZfsVolume` | `invoke` | Same. Also the path deploy uses. |
| `RemoveZfsVolume` | `invoke` | Same, and destructive. |

Three things about this surface:

- **Ensure, not create.** `EnsureZfsVolume` creates, adopts, or re-quotas. `ployz storage quota set` is the same call with a different number. Idempotent means deploy can call it every time.
- **The client never learns the pool's name.** It sends `{ name, quota_bytes }`. The daemon owns pool name, dataset path, mountpoint, and the Docker handle. `--from tank` therefore costs the client nothing.
- **No pool destroy RPC.** See "What I refuse".

`ZfsVolumeEnsured` returns the `DockerVolume` (so deploy has a handle) plus `dataset`, `quota_bytes`, `used_bytes`. `MachineStorage` returns pool facts plus one row per managed dataset. Both are Live Observation of one Machine, never merged into a cluster figure.

## How a dataset becomes a mount

```
compose: data / x-zfs 10G
        │
        ▼
EnsureZfsVolume ──► zfs create ployz/volumes/data
                    refquota=10G
                    mountpoint=/var/lib/ployz-zfs/volumes/data
                         │
                         ▼
                    docker volume create data
                      driver=local
                      o=bind device=/var/lib/ployz-zfs/volumes/data type=none
                      labels: ployz.zfs.dataset, ployz.zfs.quota
                         │
                         ▼
                    container mounts the named volume "data"
```

The Docker Volume is the handle; the dataset is the storage; `refquota` is the bound. This is why the design is small: the existing planner already pins a Service to a Machine that has a named volume of that name (`volume_matches` treats a spec with no explicit driver as matching any observed driver), so placement, `volume ls`, and the whole Docker Volume vocabulary come free. The labels make a plain `ListVolumes` observation enough to see that a volume is managed and what its quota is.

## Deploy path

Client-orchestrated as always: snapshot → plan → RPCs. Partial results are normal.

1. **Load.** `x-zfs` becomes a field on the existing `VolumeSource::Named`, not a new variant. The volume stays a named volume everywhere downstream.
2. **Snapshot.** Only when the project has at least one `x-zfs` volume, the Deploy Snapshot also collects `InspectZfsStorage` per Machine. Projects without `x-zfs` pay nothing.
3. **Plan.** Placement pinning is unchanged. Creation emits `DeployOperation::EnsureZfsVolume { machine_id, name, quota }` instead of `CreateVolume`, in the same position — before container create.
4. **Preflight refusals**, all decided from that one snapshot:
   - target Machine advertises no `ployz.zfs.volume.ensure.v1` → refuse, name the fix (`ployz storage pool create --machine m2`). Never silently degrade to an unbounded Docker Volume.
   - new volume whose quota exceeds the pool's *free* space → refuse. A guaranteed runtime ENOSPC caught while a human is watching.
   - existing volume whose requested quota is below current `used` → refuse, print the used size. Setting `refquota` under `used` is legal in ZFS and instantly wedges the app's writes.
5. **Execute.** `invoke` per target. A failure on m2 leaves m1's completed prefix in place and lands in the Deploy Outcome. **A failed deploy never destroys a dataset.** The unexecuted suffix is a suffix, not a rollback.

## Pool defaults and the math

`ployz storage pool create --machine m1` with no `--size`:

```
reserve = clamp(0.20 × disk_size, 10 GiB, 64 GiB)
pool    = free_space − reserve
refuse if pool < 8 GiB
```

`reserve` is what the host filesystem keeps for the OS, the ployz state dir, journald, and the operator's ability to breathe. It is a percentage on small disks because 10 GiB of 32 GiB matters, and a flat cap on large disks because nobody needs 800 GiB of "headroom" on a 4 TB box.

Scenarios, assuming ~4 GiB already used by OS and Docker:

| Disk | Free | Reserve | Default pool | Verdict |
| --- | --- | --- | --- | --- |
| 20G | 16G | 10G | 6G → **refused** | Below the floor. Message: this Machine is too small for managed storage; pass `--size` explicitly if you disagree. |
| 32G | 28G | 10G | 18G | Works. Host keeps 10G, and Docker is inside the pool, so 10G is genuinely spare. |
| 80G | 76G | 16G | 60G | Comfortable. |
| 256G | 252G | 51G | 201G | Percentage still reasonable at this size. |
| 1T | ~1020G | 64G (capped) | ~956G | Cap earns its keep. |
| 4T | ~4090G | 64G (capped) | ~4026G | Ditto. |

Inside the pool, the daemon holds back a hidden `refreservation` on a dataset nobody sees: `clamp(0.10 × pool, 1 GiB, 32 GiB)`. A ZFS pool at 100% is miserable to recover — you cannot always free space without writing. This guarantees there is always room to delete, resize, or grow. `ployz storage ls` reports it as part of `USED` and never as a knob.

Growing: `ployz storage pool grow --size 200G` extends the file and expands the vdev. Shrinking is refused.

## Allocate or not: fallocate. Not close.

**Recommendation: `fallocate`. Refuse sparse. Refuse to fall back to sparse when `fallocate` is unavailable.**

Sparse `truncate` failure mode, in order:

1. Pool file is "100G". The host filesystem has 60G free. ZFS believes it has 100G and will happily accept 100G of writes.
2. Docker pulls images, a container writes a log, the host filesystem hits 100%.
3. The next dataset write fails at the vdev with EIO. There is one vdev, so there is no redundancy and no retry.
4. With `failmode=wait` (the default), the pool suspends. Every dataset hangs. Every container touching storage goes into uninterruptible sleep. The Machine is functionally dead, and `zfs list` hangs with it.
5. You cannot fix it from inside: freeing space in the pool requires writes, and writes are suspended. Recovery is free host space, then `zpool clear`.

Every quota in the design is worthless in that scenario, because the party that stole the blocks was Docker or the OS, not the app that was over quota. That is precisely the "one noisy service takes down the Machine" outcome the operator is paying us to prevent.

Preallocated failure mode:

1. Pool file occupies real blocks the moment it is created. Host free space drops **at create time, in front of the operator**, which is the only moment surprise is cheap.
2. The pool can never race Docker for host blocks, because there are no shared blocks to race for.
3. Pool full now means: the dataset over its `refquota` gets ENOSPC, that service breaks, everything else keeps running. Which is the product.

Costs, stated honestly: space is consumed while unused (that is the definition of headroom, and it is the thing being bought); create takes a moment on slow disks; `fallocate` is unavailable on a few filesystems. On that last one — refuse with `unsupported` and tell the operator to use `--from` with a real pool or pick a smaller `--size`. A best-effort sparse fallback would silently reintroduce the failure mode above on exactly the machines least able to survive it.

Also refused at create time: a pool file on a filesystem that is itself ZFS (use `--from`), on a network filesystem, or on tmpfs/overlay.

## Quotas: runtime `refquota`, not deploy-time packing

**Runtime `refquota` is the enforcement. Deploy-time packing is a warning, never a placement input.**

Placement cannot depend on aggregate free space, because there is no Cluster truth to aggregate and a Deploy is a bounded calculation over one observer-relative snapshot. Packing arithmetic would make a deploy fail because of a stale number, and would make placement unpredictable in exactly the way Ployz avoids elsewhere. The two narrow deploy-time refusals in step 4 above are not packing: they are single-Machine, single-volume, live facts.

`refquota`, not `quota`, so future snapshots do not consume the app's allowance.

| Scenario | What happens |
| --- | --- |
| Sentry fills events | Sentry's writes get ENOSPC. Postgres, builds, preview apps, and the host are untouched. `ployz storage quota set sentry-data=40G` fixes it live — no redeploy, no restart. If the pool is out of room, `ployz storage pool grow` first. |
| Postgres steady growth | Same mechanism; `ployz storage ls` shows `USED` against `QUOTA` before it bites. |
| Build cache runs away | Build cache is not a compose volume. It is inside the Docker dataset's quota, so it breaks builds and nothing else. |
| Two projects both named `data` | They share one dataset, because they already share one Docker Volume today — the Docker Volume namespace is flat and machine-local. The later deploy's quota wins, unless it would drop below `used`, which is refused. Ployz does not invent a namespace to paper over a collision it already has. |
| Five apps × 10G on an 18G pool | Allowed. Reported as `COMMITTED 50G / SIZE 18G`. The pool fills only if they actually grow, and the hidden reservation keeps recovery possible. |

## Where everything that grows lives

```
ployz                       (pool: file vdev, or adopted via --from)
└── ployz/                  canmount=off
    ├── volumes/<name>      one per Managed ZFS Volume    refquota from compose
    ├── docker              Docker data-root              refquota, default 40% of pool
    └── <reserved>          hidden refreservation         10% of pool, 1–32 GiB
```

Two quotas an operator ever thinks about: the per-volume one they wrote in compose, and the Docker one.

**Docker data-root on the pool** is what makes this one storage story rather than two. It captures, in one bounded dataset: images (Ployz pushes a new image to the Machine on every deploy and nothing prunes them — this is the second-largest grower after app data), container writable layers, container json-file logs, any future on-Machine build cache, and Docker named volumes that nobody annotated with `x-zfs`. That last one matters: **an unannotated `data:` volume is still incapable of taking the Machine down**, because it lives inside the Docker dataset's quota.

Adoption is guarded, because moving a live data-root is invasive:

- `--adopt-docker` defaults to on when the data-root is near-empty or has no containers. Then pool create moves it: stop Docker, `zfs create`, copy, rewrite `data-root` in `/etc/docker/daemon.json`, start Docker, leave the old directory renamed for the operator to delete.
- Otherwise pool create completes without adopting and prints exactly one line: `Docker data stays on the host filesystem. Run ployz storage pool create --machine m1 --adopt-docker after stopping services to move it.` Re-running later does the migration. Idempotent either way.

Not on the pool, deliberately:

- **`/var/lib/ployz`** (machine record, corrosion DB). The pool file lives under it or beside it and the daemon must read its own state before it can import a pool. Circular. Also small and slow-growing.
- **journald.** Already bounded by `SystemMaxUse` (10% of the filesystem, 4 GiB cap) without us doing anything. A dataset for it would be ceremony. Container logs are a different question and they are inside the Docker dataset.
- **Client-side build cache.** `ployz build` runs buildx on the operator's laptop and pushes via unregistry. There is no build cache on the Machine today. If a remote builder ever lands, it gets `ployz/build` with its own quota, and the rule above ("everything that grows is a dataset with a quota") already tells you where to put it.

The pool file defaults to `/var/lib/ployz-zfs/pool.img` — outside `/var/lib/ployz` on purpose, because `machine rm` and reset `rm -rf` the state dir, and app data must survive a reset the same way Docker Volumes do today.

## What I hide

1. The pool name, dataset paths, mountpoints, and every `zfs`/`zpool` invocation. The client sends a name and a number.
2. `refquota` vs `quota`, `compression=lz4`, `atime=off`, `xattr=sa`, `acltype=posixacl`, `ashift=12`. Chosen once, correctly, for small machines.
3. The reserved dataset.
4. The Docker named-volume handle bound to the dataset mountpoint. Humans see a volume called `data`.
5. Importing the pool at daemon start, because a file-vdev pool is not reliably imported by the distro's boot units.

## What I refuse

1. Anything cluster-wide: cluster zpool, "ZFS-enabled cluster" flag, replicated volumes, PVs. Machine-local only.
2. `driver: zfs` in compose. It would surrender the bind indirection, and with it adoption, quota changes without a redeploy, and the whole "it is just a Docker Volume" property.
3. Sparse pool files, and silent sparse fallback.
4. Dedup, `sync=disabled`, and arbitrary ZFS property passthrough from compose. `recordsize` is a real Postgres win and a named future knob, not a knob today.
5. Shrinking a quota below `used`; shrinking a pool; destroying a pool over RPC (`zpool destroy` plus `rm` is an operator's own decision, not one API call away); putting daemon state inside the pool; falling back to an unbounded Docker Volume when the Machine has no pool.

## How the deferred parts would fit

Out of this cut, and the design does not pay for them, but the shape leaves room: one dataset per volume with `refquota` (not `quota`) is exactly the substrate snapshots want, so `ployz storage snap` is a dataset operation with no model changes. Moving a volume to another Machine is `zfs send` into the target's `ployz/volumes/<name>`, then letting the existing placement pin follow the volume — the planner already pins a Service to the Machine that has the named volume, so "move the data" and "move the Service" are the same act.

## Next

Read this, then argue with exactly one section: `fallocate`, the default-size formula, or Docker data-root adoption. Those are the three decisions that are expensive to change later.
