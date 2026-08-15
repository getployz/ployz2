Status: ready-for-agent

# Machine Pool and optional volume quotas

## Problem Statement

An operator running several apps on a few Ubuntu or Debian Machines can have one Service fill the disk and take the others down with it. They want a bound on a named volume (Sentry fills its 10G, only Sentry breaks) without Kubernetes, without a cluster storage product, and without inventing a second mount kind. Small VPSs cannot run ZFS. Docker named volumes today have no quota. They also want the option, later, to put Docker's own data-root on that same store so images and logs cannot eat the host — but that is a separate, more invasive choice, not the price of getting quotas.

## Solution

At `machine init` and `machine add`, the operator opts into a **Machine Pool**: ZFS, or one preallocated ext4 image with project quotas enabled and no limits yet. That is the future-proofing. Quotas stay optional. Bounded Docker (data-root on the pool) is a second, off-by-default opt-in.

A **Provisioned Volume** is declared in compose under `x-volumes`, not under `volumes`. It is still a Docker Volume on that Machine, with a maximum size fixed in kind at first create. A name listed only under `volumes` stays a Plain Docker Volume. Deploy refuses a Provisioned Volume on a Machine with no pool. Compose never names a filesystem.

No cluster-wide default, no copying of previous Machines' answers, no stored Cluster policy. Each Machine is asked or flagged on its own.

After the Machine has joined, the daemon manages the pool and Provisioned Volumes over the existing Machine RPCs. SSH is not required for later create, grow, or quota ensure.

## User Stories

1. As an operator, I want `machine init` on a TTY to ask me `zfs`, `ext4`, or `none`, so that I choose the disk tool while I am already setting up the Machine.
2. As an operator, I want that list to always show all three, even when `zfs` will fail, so that I see the real tools rather than a filtered menu.
3. As an operator, I want Enter on that pick to abort, so that I do not silently get a store I did not mean.
4. As an operator, I want `--storage zfs|ext4|none` to skip the pick, so that scripts do not hang.
5. As an operator running `--yes` or without a TTY, I want a missing `--storage` to mean `none` and print one line that volumes are unbounded and retrofitting is expensive, so that headless `machine init` keeps working and the choice is not silent.
6. As an operator whose pick fails (no module, leftover under about 8G, no loop, OpenVZ), I want init to fail with that reason, so that I am not left thinking I have a store. Live Docker does not fail a pool pick; it only fails bounded Docker.
7. As an operator on Ubuntu who picks `zfs`, I want the installer to apt-install ZFS binary packages, so that I do not do that by hand on an empty Machine.
8. As an operator on Debian who picks `zfs` without the module, I want init to fail with the one apt/DKMS line, so that Ployz never compiles a kernel module for me and never silently falls back to ext4.
9. As an operator who picks `zfs` on a Machine that already has a zpool, I want Ployz to use that pool, so that I do not get a file-vdev beside a real pool.
10. As an operator with several zpools, I want `--from` to choose one, so that Ployz does not guess.
11. As an operator who picks `ext4`, I want one fallocate'd ext4 image with project quotas turned on at mkfs, so that I can set per-directory bounds later without remounting the VPS root.
12. As an operator who picks `none`, I want today's unbounded Docker, so that a tiny or container VPS still joins a Cluster.
13. As an operator who picked `zfs` or `ext4` on a TTY, I want a second question "put Docker's data-root on the store?", so that moving the data-root is a conscious extra.
14. As an operator on that question, I want `y` to opt in, `n` or Enter to mean no, and init to continue, so that Enter does not abort the whole command.
15. As an operator, I want `--docker-on-store` off unless I set it, including under `--yes`, so that Docker stays on the host unless I ask.
16. As an operator passing `--docker-on-store` with `--storage none`, I want an error, so that I cannot ask for a combination that does not exist.
17. As an operator with `dockerd` already running, or `--no-install`, I want `--docker-on-store` refused, so that init never moves a live data-root.
18. As an operator in that situation who still wants a pool, I want `--storage zfs|ext4` without `--docker-on-store` to create the pool and leave Docker alone, so that I can get Provisioned Volumes without touching the data-root.
19. As an operator who picked `zfs` or `ext4` on a TTY without `--storage-size`, I want a size menu based on leftover space on the **target Machine** (25%, 50%, all leftover, or type a size), so that I pick a number from real free space instead of a guessed default.
20. As an operator on that menu, I want leftover to be free minus a host reserve (`clamp(20% of disk, 10G, 64G)`), option 3 never taking the reserve, a typed size above leftover refused, and Enter aborting, so that the host is not fallocate'd full by accident.
21. As an operator using `--yes` or no TTY who picked `zfs` or `ext4`, I want a missing `--storage-size` to be a usage error naming the flag, so that scripts never get a guessed pool size.
22. As an operator passing `--storage-size`, I want the menu skipped, so that scripts pin the size.
23. As an operator reusing an existing zpool, I want `--storage-size` and the menu ignored, so that Ployz does not resize a pool it did not create.
24. As an operator, I want the pool file or image preallocated with fallocate, sparse refused, so that the store cannot race the host filesystem for blocks.
25. As an operator, I want `machine add` to use the same `--storage` / `--docker-on-store` / `--storage-size` flags and the same TTY picks as init, so that a second Machine is not a different product.
26. As an operator, I do not want the second Machine to inherit the first Machine's answers, so that a Debian VPS is not silently given Ubuntu's ZFS choice.
27. As an operator, I want `--storage` and friends on `machine init`/`add` forwarded to the installer as argv, so that I keep passing CLI flags and the script still runs pool creation before Docker.
28. As an operator, I want a named volume listed only under `volumes:` to stay a Plain Docker Volume, so that existing compose files keep working.
29. As an operator, I want Provisioned Volumes declared in a sibling map `x-volumes`, so that Plain and Provisioned cannot share a compose key the way `ports` and `x-ports` cannot.
30. As an operator, I want the same name in both `volumes:` and `x-volumes:` to be a compose load error, so that kind is chosen in one place.
31. As an operator, I want `x-volumes.data` to accept `10G` or `{size: 10G}`, so that the shape matches `x-caddy`.
32. As an operator, I want `{}`, a missing `size`, unknown object keys, and a bare `10` to be load errors, so that typos and ten-byte quotas do not silently ship.
33. As an operator, I want legal size suffixes to be `k`/`m`/`g`/`t` (1024-based, `kb`/`kib` and friends included as today), so that `1T` works and units are not guessed as bytes.
34. As an operator, I want a service named-volume mount to resolve against `volumes:` **or** `x-volumes:` (not both), so that a misspelled `x-volums` fails as "volume not found in project volumes" instead of creating an unbounded volume.
35. As an operator, I want `x-volumes` only at the compose top level, never on a service mount, so that storage is a property of the volume name.
36. As an operator, I want no `external` key under `x-volumes`, so that Ployz always owns create/ensure and leftover Plain volumes stay a name clash.
37. As an operator, I want compose to never say `x-zfs` or `driver: zfs`, so that one compose file can target Machines with different tools.
38. As an operator deploying `x-volumes` to a Machine whose successful inspect reports no pool, I want that volume's deploy to refuse and name `machine pool create` or dropping `x-volumes`, so that I never get a silent unbounded volume.
39. As an operator, I want a Provisioned Volume to appear in `ployz volume ls` as the Docker Volume it is, with quota and used columns, so that placement pinning and inspect keep working.
40. As an operator, I want two compose projects both naming `data` to share one Docker Volume on that Machine, so that Ployz does not invent a namespace Docker does not have.
41. As an operator of a second project that declares a different size for that shared `data`, I want last writer to win (live ensure), so that raise-in-compose keeps working when names are shared.
42. As an operator who declares Plain `data` while a Provisioned `data` already exists (or the reverse), I want a deploy name clash, so that kind stays sticky and leftover Plain Sentry cannot become provisioned by editing compose.
43. As an operator raising `10G` to `20G` on an already Provisioned Volume, I want a live ensure with no container recreate, so that the fix for a full Sentry is one deploy or `ployz volume quota`.
44. As an operator, I want that ensure refused if the new quota is below current used, so that I do not instantly ENOSPC the app.
45. As an operator, I want `ployz volume quota` to ensure a new size only on an already Provisioned Volume, so that CLI cannot convert a Plain volume either.
46. As an operator, I want `ployz machine ls` to show per-Machine tool, pool size, used, and free, so that I can see who is about to fill up without a new top-level noun.
47. As an operator, I want `ployz machine pool create` and `ployz machine pool grow` after join, as daemon RPCs, so that I can opt in or expand without SSH.
48. As an operator, I want `pool grow` to take an absolute new size, refuse shrink, and refuse above live leftover, so that I cannot quietly compact or overshoot.
49. As an operator, I want later `pool create` to install no packages, so that a running Sentry is not under a DKMS compile; print the apt line instead.
50. As an operator, I want later `pool create` to still refuse `--docker-on-store` while `dockerd` is running, so that this cut never stop-and-copies a live data-root.
51. As an operator, I want no default quota on Docker's data-root, so that the pool/image size is the only budget until I set volume quotas.
52. As an operator on ext4, I want all Provisioned Volume dirs (and optional Docker data-root) inside that one image, so that I have one loop device and Linux project quotas, not a loop farm.
53. As an operator on ZFS, I want each Provisioned Volume to be a dataset with `refquota` bound to a Docker named volume, so that Sentry's ENOSPC is ZFS's ENOSPC.
54. As an operator who SSHs into the Machine, I want ordinary Linux tools (`zfs list`, `repquota -P`, `df` on the mount) to tell the truth, so that Ployz is not a layer I must go through to debug disk.
55. As an operator, I want a failed Deploy to never destroy a volume, so that a suffix failure leaves completed work in place (Partial Result).
56. As an operator, I want snapshots, send/recv, btrfs, shrinking a pool, converting Plain to Provisioned, and Corrosion volume rows out of this cut, so that this ships as quotas on a chosen Linux tool.

## Implementation Decisions

- Two operator opt-ins, never bundled: (1) Machine Pool tool `zfs|ext4|none`; (2) bounded Docker, off unless `--docker-on-store`.
- No magic: do not copy previous Machines' answers, do not write a Cluster storage policy, do not store defaults in the client config. Caddy's "copy newest running Caddy" pattern is not used here. Do not store volume rows in Corrosion in this cut; last-seen fallback for offline Machines is a later grill.
- Glossary: **Provisioned Volume**, not Managed Volume. Kind (Plain vs Provisioned) is sticky at first create. Changing the declaration does not convert existing data.
- Init and add share `--storage`, `--storage-size`, and `--docker-on-store`. TTY picks run only when the corresponding flag is absent and a TTY is present.
- No TTY / `--yes` without `--storage`: treat as `none` and print one line that there is no Machine Pool, named volumes are unbounded, and retrofitting is expensive. Do not error.
- No TTY / `--yes` with `--storage zfs|ext4` and no `--storage-size`: usage error naming `--storage-size`. Reusing an existing zpool does not require `--storage-size`.
- TTY Enter: on the tool list and the size menu, empty Enter aborts. On bounded Docker, empty Enter means no (init continues). That is not the destructive `confirm()` helper (where anything but `y` aborts the command) and not the context picker (empty takes a default).
- The client forwards `--storage` / `--storage-size` / `--docker-on-store` to the installer as argv (`bash -s -- --storage …`). The installer takes no TTY of its own. Pool creation runs before Docker is installed or first-started. The installer may apt-install ZFS **binary** packages on Ubuntu when the operator picked `zfs`. It never DKMS-compiles. The daemon never installs packages. `auto` as a storage value does not exist.
- Leftover for the size menu and for refusing an oversized `--storage-size`: `reserve = clamp(20% of disk, 10G, 64G)`; `leftover = free − reserve`. Refuse a new file-vdev or ext4 image if leftover < ~8G. Menu options are 25%, 50%, and 100% of leftover, plus a typed size. There is no default pool size formula.
- `machine pool grow` takes an absolute new size, refuses shrink, refuses above live leftover.
- Preallocation is `fallocate`. Sparse is refused, including as a fallback.
- ZFS: if a usable zpool already exists, use it (`--from` only when several exist). Otherwise a file vdev. Datasets with `refquota` for Provisioned Volumes. Docker data-root becomes a dataset only when bounded Docker was opted in.
- ext4: always one Ployz image, never the VPS root. `mkfs` enables project quotas before first mount. A new Provisioned Volume is an empty directory; its project ID is set before the app mounts. This cut does not walk inodes on a dirty tree (migrate/convert is out of scope). One loop device. Nested accounting is one project ID per inode.
- Compose sibling map, same exclusion as `ports` / `x-ports`:

```yaml
volumes:
  logs:

x-volumes:
  data: 10G          # or {size: 10G}

services:
  sentry:
    volumes:
      - data:/var/lib/sentry
      - logs:/var/log/sentry
```

- `x-volumes` keys are volume declarations. Service mounts resolve against the union of `volumes` and `x-volumes`. Same name in both maps is a load error. No `external` key under `x-volumes` (unknown key). No new `VolumeSource` variant. Placement stays the existing named-volume pin.
- Leftover Plain Docker volume with the same name as an `x-volumes` entry is a deploy name clash, not an apply-quota. The reverse (Plain declaration vs live Provisioned) is the same clash. The operator uses a new name. No migrate command in this cut.
- Two projects sharing Docker name `data`: same kind and a new size is last-writer-wins ensure. Kind mismatch is a clash.
- Raise on an already Provisioned Volume is live ensure (ZFS `refquota`, ext4 `setquota`). Quota/size is excluded from spec comparison so a size-only edit does not recreate containers. Shrink below used is refused.
- CLI: no `ployz storage` noun. `ployz volume quota`, quota/used columns on `volume ls`, pool columns on `machine ls`, `ployz machine pool create|grow`. A new top-level noun does not fit the CLI deviations ledger; a new subcommand under an existing noun may still need a one-line deviation when implemented.
- After join, pool create/grow and quota ensure are daemon RPCs. `ployz.service` must allow them: `PrivateMounts=no`, and write access for the pool path, `/etc/docker`, and `/etc/systemd`. `ProtectSystem=full` as shipped today is incompatible with that.
- Deploy Snapshot always gathers pool Live Observation (same fan-out shape as volume listing). Print a pool-inspect warning only when this deploy has at least one Provisioned Volume. A failed inspect is treated like today's failed volume list: warning, then proceed as if that Machine had no pool. Only a **successful** "no pool" is a true none; the warning path can therefore mis-say "create a pool" when the Machine simply did not answer. That matches existing volume behaviour and is accepted for this cut.
- Preflight when the deploy has Provisioned Volumes: successful no-pool → refuse that volume; new quota larger than pool free → refuse; quota below used → refuse. Thin commit on ZFS is allowed. On ext4, the image size is real; over-commit of project quotas inside the image is allowed until they grow (same as `refquota`).
- Ensure, not create, for pool and for Provisioned Volumes. `volume quota` is the same ensure with a new number. No pool-destroy RPC.
- Capabilities advertised when a store is present. Request types do not name ZFS vs ext4; the Machine reports the tool as Live Observation.
- Boot: a generated systemd `.mount` unit for the pool, `RequiresMountsFor=` on `docker.service` via a drop-in, and a sentinel file inside the pool that the daemon checks before reporting a pool present. Permission bits on an unmounted mountpoint are not a mitigation (`dockerd` is root).
- `--docker-on-store` is refused when `dockerd` is observed running, not only when `--no-install` was passed (`install_docker` returns early if Docker is already present).

## Testing Decisions

Test external behaviour: CLI flags and prompts, compose load errors, deploy refuse/accept, and daemon ensure/inspect outcomes. Do not test `zfs` argv strings, project-ID integers, or loop device numbers.

**Seams (three, all existing):**

1. **Machine init/add + installer** — the provisioning path that already installs Docker. Assert flag parsing, no-TTY/`--yes` → `none` plus the printed line, TTY tool-pick abort, size menu leftover math and Enter abort, bounded-Docker Enter = no, `--docker-on-store` rejected with `--storage none` and with live `dockerd`/`--no-install`, too-small/no-loop/OpenVZ failure messages, installer argv, pool before Docker. Prefer CLI-shape tests and the existing install-script tests (including the loosened systemd unit).
2. **Compose load** — the path that already rejects `ports` plus `x-ports` and unknown `x-caddy` keys. Assert `x-volumes` parse (scalar/object/empty/missing size/unknown key/bare number/`external`/same name in both maps/undeclared service mount), and that `x-volumes` is invalid on a service.
3. **Named volume deploy** — the path that already pins placement on named volumes (`CreateVolume` when missing). Assert leftover Plain vs Provisioned name clash, raise does not recreate, last-writer size ensure, no-pool refuse, quota below used refuses, `volume ls` still shows a Docker Volume, pool inspect always gathered, pool warning only when the deploy has `x-volumes`.

Daemon storage RPCs are tested the way volume RPCs are tested today (in-process daemon + request/response), not by parsing ZFS output. Layer-3 volume tests are prior art for CreateVolume in a plan.

A good test is one the operator could have seen: a flag error, a compose error, a Deploy Outcome, `volume ls` / `machine ls` rows. A bad test is one that opens a dataset path or a loop device number.

## Out of Scope

- Cluster-wide pool, "ZFS-enabled cluster", copying storage choices from other Machines, defaults in client config
- `driver: zfs`, compose keys that name a filesystem, CSI/storage-class/"capabilities" in compose
- Sparse pool files, silent sparse fallback, automatic pool growth, shrinking a pool, pool destroy over RPC, a default pool-size formula
- Installing kernel modules from the daemon; DKMS compile from the installer
- Default-on store; default-on bounded Docker; default Docker data-root quota
- Snapshots, send/recv, moving a volume between Machines, btrfs, recordsize knobs
- Enabling project quotas on the VPS root
- Per-volume loop images
- OpenVZ/unprivileged LXC getting a fake quota
- Reconciling or auto-healing storage
- Converting Plain to Provisioned (migrate / stop-and-copy / in-place inode walk)
- Bounded Docker after `dockerd` already exists
- `external` Provisioned Volumes
- Corrosion / replicated volume rows as an offline fallback
- A top-level `ployz storage` noun

## Further Notes

Uncloud's bar: proven Linux tools, imperative CLI, Compose as Compose. Ployz should wire those tools, not become a storage product. After join, the management plane is the daemon, not SSH; `zfs list` / `repquota -P` / `df` remain the debug tools when someone does SSH in.

The expensive later path is opting into bounded Docker after Docker already exists: stop-and-copy. That path is out of this cut. Init is cheap when Docker does not exist yet. A pool without moving Docker is still available then and later.

ext4 project quotas can be walked out of by `chattr -p` from a root container. That is accepted as the poorer tool's tax. ZFS `refquota` does not have that hole.

Failed pool inspect treated as no pool can print the wrong fix ("create a pool") when the Machine did not answer. That is the same shape as today's failed volume list, accepted here, not a TODO on the volume path.
