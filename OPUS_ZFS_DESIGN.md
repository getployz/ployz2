# Managed storage for Ployz

Design only. No implementation. Target Machines: Ubuntu and Debian, systemd, Docker with the containerd image store — what `scripts/install.sh` sets up.

## The shape, in five lines

1. A human writes `data:` with `x-quota: 10G` in compose, deploys, and gets a bounded volume. On a ZFS Machine that is a dataset with `refquota=10G`; on a Machine without ZFS it is a preallocated 10G ext4 image. The compose file does not know which.
2. The bounded volume is handed to Docker as an ordinary named volume bound to its mountpoint. Everything that already works — placement pinning, `ployz volume ls`, `docker inspect` — keeps working untouched.
3. The pool is machine-local and lives on preallocated files on whatever filesystem the Machine already has. No dedicated disk. The default claims a modest slice, not the disk; `ployz storage pool grow` is a normal, online, boring command.
4. Docker's data-root moves onto the same pool with its own quota. So *everything that grows* is inside one budget, and nothing can eat the host filesystem.
5. Quotas are enforced at runtime by the filesystem, never by deploy-time packing arithmetic. Sentry filling its quota breaks Sentry.

Domain terms: **Managed Volume** for the bounded volume, **Machine Pool** for the machine-local storage budget, **Storage Backend** for the per-Machine choice between ZFS and preallocated images. `CONTEXT.md` currently reserves the narrower *Managed ZFS Volume*; that entry wants widening when this lands, and this document does not edit it. None of these are Cluster state.

## Compose

```yaml
services:
  web:
    image: sentry:latest
    volumes:
      - data:/var/lib/sentry

volumes:
  data:
    x-quota: 10G
```

Long form, for when `quota` needs company later:

```yaml
volumes:
  data:
    x-quota:
      size: 10G
```

Scalar-or-object mirrors `x-caddy` exactly (`RawCaddy::String | Object`). Unknown keys inside the object are an error, not a shrug — `x-pre_deploy`'s "unknown attributes ignored" TODO is a wart, not a pattern.

**The key is `x-quota`, not `x-zfs`.** One compose file is deployed by one client to several Machines that need not have the same storage backend, so a key naming the filesystem would be a landmine: it would either fail on the Machine without ZFS or, worse, quietly mean nothing there. The human is expressing a bound, and a bound is what every backend implements. If the spelling must stay `x-zfs`, nothing else in this design changes and the word is simply wrong on half the fleet.

Rules:

- `x-quota` is only valid on a top-level volume, never on a service mount. Storage is a property of the volume, not of who mounts it.
- A volume without `x-quota` stays a plain Docker Volume. It is still bounded, because it lives inside the Docker quota (see below). Deploy prints one warning line per unannotated volume when the target Machine has a pool.
- `x-quota: none` is the explicit opt-out, for Machines that cannot enforce anything (see "Machines that cannot run ZFS"). It lives in the compose file, where it is greppable and reviewable, rather than in a CLI flag that leaves no trace.
- No `driver: zfs`, ever. See "What I refuse".

## Operator CLI

Same targeting as `volume create --machine`: one Machine per mutation, fan-out for reads, Partial Result on fan-out.

```
ployz storage pool create --machine m1 [--size 100G] [--from tank] [--path DIR]
                                       [--backend auto|zfs|file] [--adopt-docker]
ployz storage pool grow   --machine m1 --size 200G
ployz storage ls          [--machine m1 ...]
ployz storage quota set   data=20G --machine m1
ployz storage quota set   docker=48G --machine m1
ployz storage rm          data --machine m1 [--yes]
```

`ployz storage ls`:

```
MACHINE	BACKEND	POOL	BACKING	SIZE	USED	FREE	COMMITTED
m1	zfs	ployz	file:/var/lib/ployz-storage/pool.img	100G	15G	85G	50G
m2	zfs	tank	adopted	1.8T	400G	1.4T	120G
m3	file	-	dir:/var/lib/ployz-storage	60G	34G	26G	34G

MACHINE	VOLUME	QUOTA	USED
m1	data	10G	2.3G
m1	sentry-data	40G	38G
m1	<docker>	40G	12G
```

`COMMITTED` above `SIZE` is legal and printed on the ZFS backend, not blocked: that is thin provisioning working as intended. On the file backend `COMMITTED` equals `USED` by construction, because every quota is a preallocated image — the same column, a different truth, and the operator can see which from the `BACKEND` column.

`ployz volume ls` output does not change. Managed Volumes appear there as the Docker Volumes they are.

## RPC

Four catalog rows. Requests live in `ployz-core/src/rpc/storage.rs` next to `rpc/docker.rs`.

```
EnsureStoragePool:   (ensure_storage_pool,  "EnsureStoragePool",  EnsureStoragePoolRequest,  "ensure_storage_pool",  StoragePoolEnsured)
EnsureManagedVolume: (ensure_managed_volume,"EnsureManagedVolume",EnsureManagedVolumeRequest,"ensure_managed_volume",ManagedVolumeEnsured)
InspectStorage:      (inspect_storage,      "InspectStorage",     InspectStorageRequest,     "inspect_storage",      MachineStorage)
RemoveManagedVolume: (remove_managed_volume,"RemoveManagedVolume",RemoveManagedVolumeRequest,"remove_managed_volume",ManagedVolumeRemoved)
```

Capabilities, advertised only when the daemon finds a usable backend — gated the way volume capabilities are gated on `self.containers.is_some()`:

```
ployz.storage.pool.ensure.v1
ployz.storage.volume.ensure.v1
ployz.storage.inspect.v1
ployz.storage.volume.remove.v1
```

**The capability names do not mention the backend, and neither does any request.** Which backend a Machine uses is an observation it reports, not a dialect the client speaks. That is the whole reason the non-ZFS story below costs the client nothing: a Machine that gains ZFS later starts reporting `backend: zfs` and no compose file, RPC, or CLI invocation changes.

`call` vs `invoke`:

| RPC | Style | Why |
| --- | --- | --- |
| `InspectStorage` | `call`, fanned out per Machine into a `PartialResult` like `Client::list_volumes` | Read. Retryable. Some Machines will be down; that is a Partial Result, not a failure. |
| `EnsureStoragePool` | `invoke` with `MachineSelector` + `TARGET_RPC_TIMEOUT` | One-target mutation, exactly like `CreateVolume`. Not retried behind the operator's back. |
| `EnsureManagedVolume` | `invoke` | Same. Also the path deploy uses. |
| `RemoveManagedVolume` | `invoke` | Same, and destructive. |

Three things about this surface:

- **Ensure, not create.** `EnsureManagedVolume` creates, adopts, or re-quotas. `ployz storage quota set` is the same call with a different number, and `ployz storage pool grow` is `EnsureStoragePool` with a larger one. Idempotent means deploy can call it every time.
- **The client never has to say which backend or pool it means.** It sends `{ name, quota_bytes }`. The daemon owns backend selection, pool name, dataset or image path, mountpoint, and the Docker handle. Backend and pool travel in the other direction only, as facts `ployz storage ls` prints. `--from tank` therefore costs the client nothing.
- **No pool destroy RPC.** See "What I refuse".

`ManagedVolumeEnsured` returns the `DockerVolume` (so deploy has a handle) plus `quota_bytes` and `used_bytes`. `MachineStorage` returns `backend`, pool facts, and one row per managed volume. Both are Live Observation of one Machine, never merged into a cluster figure.

## How a quota becomes a mount

```
compose: data / x-quota 10G
        │
        ▼
EnsureManagedVolume
        │
        ├─ backend zfs ──► zfs create ployz/volumes/data, refquota=10G
        │
        └─ backend file ─► fallocate 10G volumes/data.img, mkfs.ext4, mount -o loop
        │
        ▼
   mountpoint /var/lib/ployz-storage/volumes/data
        │
        ▼
   docker volume create data
     driver=local
     o=bind device=/var/lib/ployz-storage/volumes/data type=none
     labels: ployz.storage.backend, ployz.storage.quota
        │
        ▼
   container mounts the named volume "data"
```

The Docker Volume is the handle; the dataset or image is the storage; `refquota` or the image size is the bound. This is why the design is small: the existing planner already pins a Service to a Machine that has a named volume of that name (`volume_matches` treats a spec with no explicit driver as matching any observed driver), so placement, `volume ls`, and the whole Docker Volume vocabulary come free. The labels make a plain `ListVolumes` observation enough to see that a volume is managed and what its quota is.

One hazard both backends share: if Docker starts a container before the daemon has mounted the dataset or image, the container writes into the bare mountpoint directory and the data silently goes somewhere else. Two mitigations, both invisible: the daemon orders itself `Before=docker.service`, and an unmounted mountpoint is left mode `0500` and root-owned so a container that beats the mount fails loudly instead of diverging.

## Deploy path

Client-orchestrated as always: snapshot → plan → RPCs. Partial results are normal.

1. **Load.** `x-quota` becomes a field on the existing `VolumeSource::Named`, not a new variant. The volume stays a named volume everywhere downstream.
2. **Snapshot.** Only when the project has at least one `x-quota` volume, the Deploy Snapshot also collects `InspectStorage` per Machine. Projects without `x-quota` pay nothing.
3. **Plan.** Placement pinning is unchanged. Creation emits `DeployOperation::EnsureManagedVolume { machine_id, name, quota }` instead of `CreateVolume`, in the same position — before container create.
4. **Preflight refusals**, all decided from that one snapshot:
   - target Machine advertises no `ployz.storage.volume.ensure.v1` → refuse, name the fix (`ployz storage pool create --machine m2`). Never silently degrade to an unbounded Docker Volume.
   - new volume whose quota exceeds the pool's *free* space → refuse. A guaranteed runtime ENOSPC caught while a human is watching. On the file backend this is not merely a good idea: free space is the allocation budget, so the refusal is the enforcement.
   - existing volume whose requested quota is below current `used` → refuse, print the used size. Setting `refquota` under `used` is legal in ZFS and instantly wedges the app's writes; shrinking an ext4 image needs it offline.
5. **Execute.** `invoke` per target. A failure on m2 leaves m1's completed prefix in place and lands in the Deploy Outcome. **A failed deploy never destroys a volume.** The unexecuted suffix is a suffix, not a rollback.

## Pool defaults and the math

Preallocation changes what a default *means*. With a sparse file, claiming most of the disk is free and the only sane default. With `fallocate`, every byte of the default is a byte spent the moment the operator runs one command, and the earlier formula's answer on a 2 TB disk — hand ZFS 1.9 TB up front — is wrong: it leaves `df` pinned at 97% forever, makes every host-level backup and `rsync` step over a two-terabyte file, and forecloses any future use of the disk that is not Ployz. **So the default gets conservative and `pool grow` gets promoted from emergency lever to the normal way pools reach their eventual size.**

`ployz storage pool create --machine m1` with no `--size`:

```
reserve = clamp(0.20 × disk_size, 10 GiB, 64 GiB)
usable  = free_space − reserve
pool    = min(usable, max(100 GiB, 0.25 × usable))
refuse if usable < 8 GiB
```

Two rules, one number: **take everything usable up to 100 GiB, and a quarter of it above that.** Small Machines get all the space that exists, because splitting 18 GiB into "pool" and "spare" is a lie told to nobody's benefit. Large Machines get a starter slice that is obviously enough to begin and obviously not the disk. The number means "the size of the pool file" on the ZFS backend and "the allocation budget for volume images" on the file backend.

`reserve` is what the host filesystem keeps for the OS, the ployz state dir, journald, and the operator's ability to breathe. It is a percentage on small disks because 10 GiB of 32 GiB matters, and a flat cap on large disks because nobody needs 800 GiB of headroom on a 4 TB box. It only ever binds on the small end now, which is the right place for it to bind.

Scenarios, assuming ~4 GiB already used by OS and Docker:

| Disk | Usable | Default pool | Host keeps | Verdict |
| --- | --- | --- | --- | --- |
| 20G | 6G | **refused** | — | Below the floor. Message: this Machine is too small for a managed pool; pass `--size` if you disagree. |
| 32G | 18G | 18G | 10G | All of it. Docker is inside the pool, so 10G of host is genuinely spare. |
| 80G | 60G | 60G | 16G | All of it. |
| 256G | 201G | 100G | 152G | Starter slice. Growing to 200G is one online command. |
| 1T | 956G | 239G | 781G | Starter slice at 25%. |
| 2T | 1980G | 495G | 1549G | The case that motivated the change: a quarter, not everything. |
| 4T | 4026G | 1006G | 3T | Same rule, no special case. |

Two adjustments to the default, both automatic:

- **Adopting Docker raises the floor.** If `--adopt-docker` will run, the default becomes `max(default, 3 × current data-root usage)`, and pool create refuses when that exceeds `usable`. A 256 GiB Machine carrying 60 GiB of images cannot start with a 100 GiB pool whose Docker dataset is capped at 40 GiB — it would refuse the migration it was told to perform.
- **Docker's share.** The Docker dataset gets 40% of the pool, floored at 8 GiB, capped at 64 GiB, and never more than half the pool. `ployz storage quota set docker=...` overrides it and is the same RPC as any other quota change.

Inside the pool, the ZFS backend holds back a hidden `refreservation` nobody sees: `clamp(0.10 × pool, 1 GiB, 32 GiB)`. A ZFS pool at 100% is miserable to recover — you cannot always free space without writing. This guarantees there is always room to delete, resize, or grow, and `ployz storage ls` reports it inside `USED`, never as a knob.

### Grow

`ployz storage pool grow --machine m1 --size 200G` takes an **absolute new size**, not a delta, matching how quotas are set everywhere else in the product. It refuses to shrink, and it refuses to exceed `free − reserve` recomputed from live `statfs` at that moment rather than from whatever was true at create time.

Growing is online and unremarkable: extend the file, then expand the vdev (ZFS) or refresh the loop device and `resize2fs` (file backend). No unmount, no container restart, no Docker restart, no downtime. On the ext4 and xfs root filesystems that Ubuntu and Debian actually ship, `fallocate` is an extent operation, so both create and grow are effectively instant regardless of size.

Three deliberate properties:

1. **Growth is safe later because Docker moved into the pool.** After adoption, host filesystem usage is nearly static — OS, journald, ployz state. The space left behind at create time is still there months later, which is precisely what makes a conservative default honest rather than a trap.
2. **Growth is never automatic.** An auto-grow rule is a reconciliation loop, and Ployz does not have those. What it has instead: every message that growing would fix prints the exact command with the number already filled in, and `ployz storage ls` warns at 80% pool usage.
3. **Growing the pool and growing a quota are different acts, and on ZFS they feel different.** Raising a `refquota` is instant metadata against shared free space; raising a quota on the file backend allocates immediately and can be refused for want of budget. Same command, and `ployz storage ls` already showed the operator which world they are in.

## Preallocation: settled, and here is why, for the record

**`fallocate`. Sparse is refused, including as a fallback when `fallocate` is unavailable.** Recorded rather than re-argued, because every other number in this document leans on it.

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

Costs, stated honestly: space is consumed while unused — that is the definition of headroom and it is the thing being bought, which is exactly why the default above buys a modest amount of it and grows later. On ext4 and xfs, `fallocate` is an extent operation, so the cost is not time; on a filesystem that cannot do it, allocation would mean writing zeros, and there Ployz refuses with `unsupported` and tells the operator to use `--from` with a real pool or a smaller `--size`. A best-effort sparse fallback would silently reintroduce the failure mode above on exactly the machines least able to survive it.

Also refused at create time: a pool file on a filesystem that is itself ZFS (use `--from`), on a network filesystem, or on tmpfs/overlay.

## Quotas: runtime `refquota`, not deploy-time packing

**Runtime `refquota` is the enforcement. Deploy-time packing is a warning, never a placement input.**

Placement cannot depend on aggregate free space, because there is no Cluster truth to aggregate and a Deploy is a bounded calculation over one observer-relative snapshot. Packing arithmetic would make a deploy fail because of a stale number, and would make placement unpredictable in exactly the way Ployz avoids elsewhere. The two narrow deploy-time refusals in step 4 above are not packing: they are single-Machine, single-volume, live facts.

`refquota`, not `quota`, so future snapshots do not consume the app's allowance.

| Scenario | What happens |
| --- | --- |
| Sentry fills events | Sentry's writes get ENOSPC. Postgres, builds, preview apps, and the host are untouched. `ployz storage quota set sentry-data=40G` fixes it live — no redeploy, no restart. If the pool is out of room, `ployz storage pool grow` first. |
| Postgres steady growth | Same mechanism; `ployz storage ls` shows `USED` against `QUOTA` before it bites. |
| Build cache runs away | Build cache is not a compose volume. It is inside the Docker quota, so it breaks builds and nothing else. |
| Two projects both named `data` | They share one volume, because they already share one Docker Volume today — the Docker Volume namespace is flat and machine-local. The later deploy's quota wins, unless it would drop below `used`, which is refused. Ployz does not invent a namespace to paper over a collision it already has. |
| Five apps × 10G on an 18G pool | On ZFS: allowed, reported as `COMMITTED 50G / SIZE 18G`. The pool fills only if they actually grow, and the hidden reservation keeps recovery possible. On the file backend: the sixth 10G volume is refused at deploy time, because the budget is real. |

## Machines that cannot run ZFS

**Yes, they get quotas. The mechanism is a preallocated ext4 image per volume, mounted over a loop device — the same `fallocate` trick as the pool file, applied one level down.** This is the `file` backend, and it is chosen automatically.

Why not the obvious alternatives, briefly, so nobody re-proposes them: **XFS project quotas** would be ideal, but Ubuntu and Debian install on ext4, and you do not get to pick the root filesystem of a VPS after the fact. **ext4 project quotas** exist and are the right shape, but enabling them needs `tune2fs -O project,quota` on an *unmounted* filesystem plus a mount-option change — a reboot into rescue on the Machine's root filesystem, which is not a thing a deploy tool may ask for. **Loop-mounted images need nothing installed**: `losetup` is util-linux, `mkfs.ext4` is e2fsprogs, both present on every Ubuntu and Debian install that can run Docker. No package, no DKMS build, no kernel headers, no reboot.

That last point is the whole argument. Getting ZFS onto a Debian Machine means contrib plus a `zfs-dkms` compile that wants headers, time, and RAM that a small VPS may not have; on Ubuntu it means `zfsutils-linux` plus a `linux-modules-extra-$(uname -r)` that some cloud kernels ship without. **The daemon probes and picks; it never installs a kernel module on the operator's behalf**, and when the operator asks for `--backend zfs` on a Machine that lacks it, the refusal prints the one `apt` line for that distribution and stops.

What the file backend bounds, and what it does not:

| | File backend |
| --- | --- |
| Per-volume writes | **Bounded.** ENOSPC inside the image, scoped to that volume. Sentry fills its quota, only Sentry breaks. |
| Docker (images, layers, container logs, unannotated volumes) | **Bounded.** The data-root is itself one preallocated image. The one storage story survives intact. |
| Host filesystem | **Bounded by the budget.** Each volume claims its bytes when it is created, and the budget refuses allocations beyond the pool size, so Ployz's total footprint is knowable at all times. |
| Raising a quota | **Live.** Extend the image, refresh the loop device, `resize2fs` online. No restart. |
| Lowering a quota | **Not possible.** Shrinking ext4 requires unmounting. Refused, same verb, different reason than ZFS. |
| Unused space in one volume | **Stranded.** No thin provisioning; 5 × 10G is 50G gone. This is the real cost and the reason ZFS stays the preferred backend. |
| Snapshots, send/recv, compression | **None.** A Machine on this backend cannot participate in the future snapshot story. |
| Inode exhaustion | **Possible.** A volume with millions of tiny files can hit ENOSPC on inodes before bytes. `mkfs.ext4` gets a bytes-per-inode setting tuned for the size, and that is as far as this cut goes. |

One honest asymmetry. On the ZFS backend the pool file is preallocated in full, so nothing can ever race the datasets for host blocks. On the file backend only the *allocated* images are preallocated — the unclaimed remainder of the budget is ordinary host free space. The consequence is mild and lands in the right place: an existing volume can never lose blocks it already owns, and if something outside Ployz eats the host filesystem, the failure is a later `fallocate` refusing to create a *new* volume. A clean refusal at create time, not a suspended pool and not a corrupted one.

It fits the product shape without bending it: compose still says `x-quota: 10G`, the volume is still handed to Docker as a named volume bound to a mountpoint, placement still pins, enforcement is still runtime ENOSPC, and `ployz storage ls` still prints `QUOTA` against `USED`. The client cannot tell, and neither can the compose file. The pool is still a pool — on this backend it is a directory plus a budget rather than shared free space, which is why `COMMITTED` equals `USED` there and why the deploy-time "quota exceeds free space" refusal becomes strict instead of advisory. Guardrail: one loop device per Managed Volume, and the daemon refuses past 64 of them on a Machine, because past that the operator has a design problem that more loop devices will not fix.

### The third tier, where the answer is no

A Machine that is itself a container — OpenVZ, or an unprivileged LXC VPS — has neither ZFS nor loop devices, and cannot mount anything. `ployz storage pool create` refuses there with the diagnosis rather than a generic error: no ZFS module, no `/dev/loop-control`, this looks like a container-based VPS, quotas are not available on this Machine.

What the operator gets on those Machines instead:

1. **Honesty at deploy time.** A volume with `x-quota` refuses to deploy. The compose file must say `x-quota: none` for that volume, which is a line in a reviewed file rather than a flag someone typed once.
2. **Observation without enforcement.** `ployz storage ls` still reports per-volume usage and host free space — a directory walk rather than a filesystem counter, so it is a periodic figure rather than an instant one. "Which service is eating the Machine" stays a question with an answer.
3. **A real fix, named.** The fix is a KVM VPS or a machine where the kernel is yours. That is a five-dollar answer, and pretending otherwise with a soft-limit warning loop would be worse than saying it.

## Where everything that grows lives

```
ZFS backend                                   file backend
ployz              (file vdev, or --from)     /var/lib/ployz-storage/
└── ployz/         canmount=off               ├── volumes/<name>.img   ext4, size from compose
    ├── volumes/<name>   refquota             ├── docker.img           ext4, 40% of budget
    ├── docker          40% of pool           └── (budget tracked, not preallocated as one file)
    └── <reserved>      10%, hidden
```

Two quotas an operator ever thinks about: the per-volume one they wrote in compose, and the Docker one.

**Docker data-root inside the pool** is what makes this one storage story rather than two, and it works identically on both backends. It captures, in one bounded place: images (Ployz pushes a new image to the Machine on every deploy and nothing prunes them — this is the second-largest grower after app data), container writable layers, container json-file logs, any future on-Machine build cache, and Docker named volumes that nobody annotated with `x-quota`. That last one matters: **an unannotated `data:` volume is still incapable of taking the Machine down**, because it lives inside the Docker quota.

Adoption is guarded, because moving a live data-root is invasive:

- `--adopt-docker` defaults to on when the data-root is near-empty or has no containers. Then pool create moves it: stop Docker, create the dataset or image, copy, rewrite `data-root` in `/etc/docker/daemon.json`, start Docker, leave the old directory renamed for the operator to delete.
- Otherwise pool create completes without adopting and prints exactly one line: `Docker data stays on the host filesystem. Run ployz storage pool create --machine m1 --adopt-docker after stopping services to move it.` Re-running later does the migration. Idempotent either way.
- The copy needs no transient host space: the destination is already allocated, and the old directory is only released afterwards, so adoption *returns* space to the host filesystem. That ordering is what lets a conservative default coexist with a fat existing data-root, provided the pool was sized against it as described above.

Not on the pool, deliberately:

- **`/var/lib/ployz`** (machine record, corrosion DB). The pool file lives under it or beside it and the daemon must read its own state before it can import a pool. Circular. Also small and slow-growing.
- **journald.** Already bounded by `SystemMaxUse` (10% of the filesystem, 4 GiB cap) without us doing anything. A dataset for it would be ceremony. Container logs are a different question and they are inside the Docker dataset.
- **Client-side build cache.** `ployz build` runs buildx on the operator's laptop and pushes via unregistry. There is no build cache on the Machine today. If a remote builder ever lands, it gets its own dataset and quota, and the rule above ("everything that grows gets a quota") already tells you where to put it.

Storage lives under `/var/lib/ployz-storage/` — the pool file, the volume images, the mountpoints — outside `/var/lib/ployz` on purpose, because `machine rm` and reset `rm -rf` the state dir, and app data must survive a reset the same way Docker Volumes do today.

## What I hide

1. **Which backend a Machine uses.** Probed, reported in `ployz storage ls`, never asked about. A Machine that gains ZFS later changes one column and nothing else.
2. The pool name, dataset and image paths, mountpoints, loop devices, and every `zfs` / `zpool` / `losetup` / `resize2fs` invocation. The client sends a name and a number.
3. `refquota` vs `quota`, `compression=lz4`, `atime=off`, `xattr=sa`, `acltype=posixacl`, `ashift=12`, ext4 bytes-per-inode. Chosen once, correctly, for small machines.
4. The reserved dataset, and the Docker named-volume handle bound to the mountpoint. Humans see a volume called `data`.
5. Importing the pool and re-attaching loop devices at daemon start, plus the `Before=docker.service` ordering and the mode-`0500` unmounted mountpoint that keep Docker from writing underneath a volume that is not mounted yet.

## What I refuse

1. Anything cluster-wide: cluster zpool, "ZFS-enabled cluster" flag, replicated volumes, PVs. Machine-local only.
2. `driver: zfs` in compose, and any compose key that names a filesystem. The human writes a bound; the Machine picks how.
3. Sparse pool files, and silent sparse fallback.
4. Installing kernel modules from the daemon, automatic pool growth, dedup, `sync=disabled`, and arbitrary filesystem property passthrough from compose. `recordsize` is a real Postgres win and a named future knob, not a knob today.
5. Shrinking a quota below `used`; shrinking a pool; destroying a pool over RPC (`zpool destroy` plus `rm` is an operator's own decision, not one API call away); putting daemon state inside the pool; falling back to an unbounded Docker Volume when the Machine cannot enforce a quota — on those Machines the compose file says `x-quota: none` or the deploy refuses.

## How the deferred parts would fit

Out of this cut, and the design does not pay for them, but the shape leaves room: one dataset per volume with `refquota` (not `quota`) is exactly the substrate snapshots want, so `ployz storage snap` is a dataset operation with no model changes. Moving a volume to another Machine is `zfs send` into the target's `ployz/volumes/<name>`, then letting the existing placement pin follow the volume — the planner already pins a Service to the Machine that has the named volume, so "move the data" and "move the Service" are the same act. Both are ZFS-backend-only, which is the honest reason ZFS stays preferred rather than merely tolerated: the file backend buys enforcement everywhere, and nothing beyond it.

## Next

Read this, then argue with exactly one section: the default-size formula, the file backend for non-ZFS Machines, or Docker data-root adoption. Those are the three decisions that are expensive to change later. `fallocate` is settled.
