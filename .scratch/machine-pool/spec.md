Status: ready-for-agent

# Machine Pool and optional volume quotas

## Problem Statement

An operator running several apps on a few Ubuntu or Debian Machines can have one Service fill the disk and take the others down with it. They want a way to put a bound on a named volume (Sentry fills its 10G, only Sentry breaks) without Kubernetes, without a cluster storage product, and without inventing a second kind of volume. Small VPSs cannot run ZFS. Docker named volumes today have no quota. They also want the option, later, to put Docker's own data-root on that same store so images and logs cannot eat the host — but that is a separate, more invasive choice, not the price of getting quotas.

## Solution

At `machine init` and `machine add`, the operator opts into a **Machine Pool**: ZFS, or one preallocated ext4 image with project quotas enabled and no limits yet. That is the future-proofing. Quotas stay optional. Bounded Docker (data-root on the pool) is a second, off-by-default opt-in.

A volume with `x-quota` is still a **Docker Volume**. Deploy refuses `x-quota` on a Machine with no pool. `ployz storage quota set` can add a bound later to a volume that already has data. Compose never names a filesystem.

No cluster-wide default, no copying of previous Machines' answers, no stored Cluster policy. Each Machine is asked or flagged on its own.

## User Stories

1. As an operator, I want `machine init` on a TTY to ask me `zfs`, `ext4`, or `none`, so that I choose the disk tool while I am already setting up the Machine.
2. As an operator, I want Enter on that pick to abort, so that I do not silently get a store I did not mean.
3. As an operator, I want `--storage zfs|ext4|none` to skip the pick, so that scripts do not hang.
4. As an operator running `--yes` or without a TTY, I want a missing `--storage` to error and tell me to pass the flag, so that CI never guesses.
5. As an operator, I want the TTY to still list `zfs` even on a Debian box that cannot load it, so that I see the real tools, and I want a failed pick to print the reason (no module, too small, OpenVZ, live Docker in the way).
6. As an operator on Ubuntu who picks `zfs`, I want the installer to apt-install ZFS binary packages, so that I do not do that by hand on an empty Machine.
7. As an operator on Debian who picks `zfs` without the module, I want init to fail with the one apt/DKMS line, so that Ployz never compiles a kernel module for me and never silently falls back to ext4.
8. As an operator who picks `zfs` on a Machine that already has a zpool, I want Ployz to use that pool, so that I do not get a file-vdev beside a real pool.
9. As an operator who picks `ext4`, I want one fallocate'd ext4 image with project quotas turned on at mkfs, so that I can set per-directory bounds later without remounting the VPS root.
10. As an operator who picks `none`, I want today's unbounded Docker, so that a tiny or container VPS still joins a Cluster.
11. As an operator who picked `zfs` or `ext4`, I want a second TTY question for bounded Docker, default no, so that moving the data-root is a conscious extra.
12. As an operator, I want `--docker-on-store` off unless I set it, including under `--yes`, so that Docker stays on the host unless I ask.
13. As an operator passing `--docker-on-store` with `--storage none`, I want an error, so that I cannot ask for a combination that does not exist.
14. As an operator using `--no-install` when Docker is already there, I want `--storage zfs|ext4` to still create the pool and leave Docker alone, so that I can get quotas without touching a live data-root.
15. As an operator using `--no-install` plus `--docker-on-store`, I want init to refuse and name the later stop-and-copy command, so that init never migrates a live data-root.
16. As an operator whose disk cannot host a pool (usable under about 8G, no loop, OpenVZ) who still picked `zfs` or `ext4`, I want init to fail with that reason, so that I am not left thinking I have a store.
17. As an operator, I want a conservative default pool size (all usable space up to 100G, a quarter of usable above that) and `--storage-size` to override, so that a 2TB box is not fallocate'd almost full.
18. As an operator, I want `ployz storage pool grow` later with an absolute new size, so that I can expand without a size prompt at init.
19. As an operator, I want the pool file or image preallocated with fallocate, sparse refused, so that the store cannot race the host filesystem for blocks.
20. As an operator, I want `machine add` to use the same `--storage` / `--docker-on-store` / `--storage-size` flags and the same TTY picks as init, so that a second Machine is not a different product.
21. As an operator, I do not want the second Machine to inherit the first Machine's answers, so that a Debian VPS is not silently given Ubuntu's ZFS choice.
22. As an operator, I want a named volume without `x-quota` to stay an ordinary Docker Volume, so that existing compose files keep working.
23. As an operator, I want `x-quota: 10G` on a top-level compose volume (scalar or `{size: 10G}` like `x-caddy`) so that Sentry's bound lives next to Sentry.
24. As an operator, I want unknown keys inside the `x-quota` object to be an error, so that typos do not silently do nothing.
25. As an operator, I want `x-quota` only on top-level volumes, never on a service mount, so that storage is a property of the volume.
26. As an operator, I want compose to never say `x-zfs` or `driver: zfs`, so that one compose file can target Machines with different tools.
27. As an operator deploying `x-quota` to a Machine with `--storage none`, I want that volume's deploy to refuse and name `storage pool create` or dropping `x-quota`, so that I never get a silent unbounded volume.
28. As an operator, I want a Managed Volume to appear in `ployz volume ls` as the Docker Volume it is, so that placement pinning and inspect keep working.
29. As an operator, I want two compose projects both naming `data` to share one Docker Volume on that Machine, so that Ployz does not invent a namespace Docker does not have.
30. As an operator, I want `ployz storage quota set data=20G --machine m1` to bound an existing volume whose data is already in the pool, so that I do not have to recreate Sentry to cap it.
31. As an operator, I want that later quota refused if it is below current used, so that I do not instantly ENOSPC the app.
32. As an operator, I want raising a quota to work live (ZFS `refquota`, ext4 `setquota`) without restarting the container, so that the fix for a full Sentry is one command.
33. As an operator, I want `ployz storage ls` to show per-Machine tool, pool size, used, free, and per-volume quota vs used, so that I can see who is about to fill up.
34. As an operator, I want no default quota on Docker's data-root, so that the pool/image size is the only budget until I set volume quotas.
35. As an operator on ext4, I want all managed dirs (and optional Docker data-root) inside that one image, so that I have one loop device and Linux project quotas, not a loop farm.
36. As an operator on ZFS, I want each quota'd volume to be a dataset with `refquota` bound to a Docker named volume, so that Sentry's ENOSPC is ZFS's ENOSPC.
37. As an operator, I want `ployz storage pool create --machine m1` on a Machine that skipped the store at init, so that I can opt in later at the cost of a stop-and-copy if I also move Docker.
38. As an operator, I want `pool create` after the fact to install no packages, so that a running Sentry is not under a DKMS compile; print the apt line instead.
39. As an operator who SSHs into the Machine, I want ordinary Linux tools (`zfs list`, `repquota -P`, `df` on the mount) to tell the truth, so that Ployz is not a layer I must go through to debug disk.
40. As an operator, I want a failed Deploy to never destroy a volume, so that a suffix failure leaves completed work in place (Partial Result).
41. As an operator, I want snapshots, send/recv, btrfs, and shrinking a pool out of this cut, so that this ships as quotas on a chosen Linux tool.

## Implementation Decisions

- Two operator opt-ins, never bundled: (1) Machine Pool tool `zfs|ext4|none`; (2) bounded Docker, off unless `--docker-on-store`.
- No magic: do not copy previous Machines' answers, do not write a Cluster storage policy, do not store defaults in the client config. Caddy's "copy newest running Caddy" pattern is not used here.
- `provisioning_flags` grow `--storage`, `--storage-size`, and `--docker-on-store`. Init and add share them. TTY picks run only when the flag is absent and a TTY is present. `--yes` or no TTY without `--storage` is usage error.
- The installer sequences store creation. It may apt-install ZFS **binary** packages on Ubuntu when the operator picked `zfs`. It never DKMS-compiles. The daemon never installs packages. `auto` as a storage value does not exist.
- Default size, when they picked `zfs` or `ext4` and omitted `--storage-size`: reserve `clamp(20% of disk, 10G, 64G)`; usable = free − reserve; pool = min(usable, max(100G, 25% of usable)); refuse if usable < 8G. `pool grow` takes an absolute new size, refuses shrink, refuses above live `free − reserve`.
- Preallocation is `fallocate`. Sparse is refused, including as a fallback.
- ZFS: if a usable zpool already exists, use it (optional `--from` only when several exist). Otherwise a file vdev. Datasets with `refquota` for quota'd volumes. Docker data-root becomes a dataset only when bounded Docker was opted in.
- ext4: always one Ployz image, never "the VPS root". `mkfs` enables project quotas before first mount. Directories get project IDs when a quota is set (including retroactively: walk existing inodes, then `setquota`; stop the container first). One loop device. Nested accounting is one project ID per inode.
- A quota'd volume is created as a Docker named volume bound to the dataset or project directory. Placement stays the existing named-volume pin. No new `VolumeSource` variant; `x-quota` is a field on named volumes.
- Deploy snapshot collects pool inspection only when the project has at least one `x-quota` volume. Preflight: no pool → refuse that volume; new quota larger than pool free → refuse; quota below used → refuse. Thin commit on ZFS is allowed. On ext4, the image size is real; over-commit of project quotas inside the image is allowed until they grow (same as `refquota`).
- Ensure, not create, for pool and for quota'd volumes. `quota set` is the same ensure with a new number. No pool-destroy RPC.
- Capabilities advertised when a store is present. Request types do not name ZFS vs ext4; the Machine reports the tool as Live Observation.
- Boot: a small oneshot ordered before Docker remounts the pool/image. The daemon stays after Docker. Unmounted mountpoints are not world-writable so Docker cannot write underneath.
- `x-quota: none` is not required; absence of `x-quota` is the unbounded named volume. A Machine with `--storage none` simply cannot satisfy `x-quota`.

## Testing Decisions

Test external behaviour: CLI flags and prompts, compose load errors, deploy refuse/accept, and daemon ensure/inspect outcomes. Do not test `zfs` argv strings or project-ID integers.

**Seams (two, both existing):**

1. **Machine init/add** — the provisioning path that already installs Docker. Assert flag parsing, no-TTY/`--yes` errors, TTY pick abort, `--docker-on-store` rejected with `--storage none`, `--no-install` plus bounded Docker refused, too-small/no-loop failure messages. Prefer CLI-shape tests and the existing install-script tests over a new harness.
2. **Compose named volume → CreateVolume deploy** — the path that already pins placement on named volumes. Assert `x-quota` parse (scalar/object/unknown key/service-mount invalid), deploy without a pool refuses, quota below used refuses, `volume ls` still shows a Docker Volume.

Daemon storage RPCs are tested the way volume RPCs are tested today (in-process daemon + request/response), not by parsing ZFS output in the test. Layer-3 volume tests are prior art for CreateVolume in a plan.

A good test is one the operator could have seen: a flag error, a compose error, a Deploy Outcome, `storage ls` rows. A bad test is one that opens a dataset path or a loop device number.

## Out of Scope

- Cluster-wide pool, "ZFS-enabled cluster", copying storage choices from other Machines, defaults in client config
- `driver: zfs`, compose keys that name a filesystem, CSI/storage-class/"capabilities" in compose
- Sparse pool files, silent sparse fallback, automatic pool growth, shrinking a pool, pool destroy over RPC
- Installing kernel modules from the daemon; DKMS compile from the installer
- Default-on store; default-on bounded Docker; default Docker data-root quota
- Snapshots, send/recv, moving a volume between Machines, btrfs, recordsize knobs
- Enabling project quotas on the VPS root
- Per-volume loop images
- OpenVZ/unprivileged LXC getting a fake quota
- Reconciling or auto-healing storage

## Further Notes

Uncloud's bar: proven Linux tools, imperative CLI, Compose as Compose, SSH in and debug with `zfs`/`repquota`/`df`. Ployz should wire those tools, not become a storage product.

The expensive later path is opting into the store (or bounded Docker) after Docker already exists: stop-and-copy. Init is cheap only when Docker does not exist yet.

ext4 project quotas can be walked out of by `chattr -p` from a root container. That is accepted as the poorer tool's tax. ZFS `refquota` does not have that hole.
