# Uncloud ZFS / storage wants

Research date: 2026-08-15.
Question: what do Uncloud users, the author, and the homelab/self-host crowd actually want from ZFS in Uncloud?

**Answer first:** Uncloud has **no ZFS product, no ZFS code, no ZFS issues, and no ZFS roadmap**. One author comment names ZFS as an *example backend* for still-local volumes with snapshots, backups, and restore-oriented replication. No GitHub user asked for ZFS by name. Adjacent wants are NFS, backups, and “restart my postgres on another machine.”

Sources: GitHub `psviderski/uncloud` issues/PRs/discussions, official docs at [uncloud.run/docs](https://uncloud.run/docs/), README, `misc/design.md`, author’s Substack, author’s HN comments. Discord exists but was not fetchable. Do **not** treat [psviderski-uncloud.mintlify.app](https://psviderski-uncloud.mintlify.app/concepts/storage) as Uncloud docs (unofficial; contradicts the support matrix).

---

## 1. What Uncloud is

Uncloud is a lightweight clustering / container orchestration tool: Docker hosts joined by a WireGuard mesh, Compose for services and volumes, no central control plane. CLI is `uc`. Official description:

> Uncloud is a lightweight clustering and container orchestration tool that lets you deploy and manage web apps across cloud VMs and bare metal with minimised cluster management overhead. It creates a secure WireGuard mesh network between your Docker hosts and provides automatic service discovery, load balancing, ingress with HTTPS, and simple CLI commands to manage your apps.

Source: [README](https://github.com/psviderski/uncloud/blob/main/README.md) (fetched 2026-08-15). Docs site: [uncloud.run/docs](https://uncloud.run/docs/). Repo: [github.com/psviderski/uncloud](https://github.com/psviderski/uncloud).

Persistent storage today is **machine-local Docker volumes** used as placement anchors, not a storage cluster.

---

## 2. Attributed ZFS / storage wants

### ZFS by name

| Who | Want | Quote | URL | Date |
|---|---|---|---|---|
| **Author** (psviderski) | Local (not distributed) “modern volumes” with snapshots, backups, and backup-oriented replication; ZFS or device mapper as examples | “What I'm thinking to focus instead is to implement more modern volumes (still local and not distributed) with snapshots, backups, and some sort of replication for backup purposes, e.g using zfs or device mapper.” | [Issue #242 comment](https://github.com/psviderski/uncloud/issues/242#issuecomment-3771471639) | 2026-01-20 |
| **Author** (same thread) | After machine/storage failure: restore the volume on another machine and recover the app | “I.e. have a single data volume + snapshots + backups + ideally close to realtime replication to another machine/location/s3. In the rare case when the machine or storage fails, it should be possible to quickly restore the volume on another machine and recover the app.” | same comment | 2026-01-20 |
| **Users** | — | **None found.** GitHub issue search, discussion search, and code search for `zfs` in `psviderski/uncloud` and `org:psviderski` returned **0 hits**. The word appears only in the #242 comment above (comment bodies are poorly indexed). | GitHub Search API, 2026-08-15 | — |

No Uncloud source, docs page, milestone, or Compose extension mentions ZFS. `website/docs/` has no volumes concept page. `storage_opt` is **not supported** ([support matrix](https://uncloud.run/docs/compose-file-reference/support-matrix/)).

### Adjacent storage wants (not ZFS)

These are the real Uncloud storage conversation. Grouped below in §3.

---

## 3. Author stated vs user asked vs inferred from docs

### Author stated

**Distributed storage is out of scope (near term).** Closing a request to replicate postgres volumes across machines (GlusterFS-like, `sync: true`):

> Distributed storage by its nature is really not simple but I want uncloud to be easy-to-comprehend and easy-to-use tool. So my plan to keep distributed storage out of uncloud scope at least for the near future. You can still manage it on your own using ceph rbd, cephfs, linstor, sshfs, nfs, gluster, etc, mount it on the host and bind mount from the host to your service containers.

[Issue #242](https://github.com/psviderski/uncloud/issues/242#issuecomment-3771471639), 2026-01-20. Closed as not planned.

**Same idea earlier, before named volumes existed** (HN, 2025-03-07):

> Currently Uncloud doesn't handle volume replication. Moreover it doesn't support regular Docker volumes yet, only mounting a host path. The reason is I didn't have time to give it proper thought on how to design volumes in a cluster context without getting into the full-blown PV support like in K8s.
>
> I suspect that I will implement support for regular local Docker volumes such that each service container will use its own volume on the machine it runs on. Uncloud won't automatically replicate data between volumes as storage replication adds significant complexity and potential failure modes. Apps that need HA such as databases can handle their own replication. I'm getting inspiration from Fly for this: https://fly.io/docs/volumes/overview/. Maybe it would make sense to implement handy commands for cloning, moving, and backing up volumes between machines, not sure yet.

[HN comment #43287424](https://news.ycombinator.com/item?id=43287424) on [Show HN: Uncloud](https://news.ycombinator.com/item?id=43285730).

**2026 “thinking about” list names modern volumes, not ZFS:**

> Some other big topics I'm thinking about for 2026:
>
> Self-managed databases as the first-class citizen (taking inspiration from Fly.io’s Postgres support)
>
> Modern persistent volumes with snapshots, incremental backups, and replication

[A year of building Uncloud](https://psviderski.substack.com/p/a-year-of-building-uncloud), 2026-01-24. This is a wish list, not a spec. ZFS is not named here.

**No Uncloud-native postgres backup yet.** After milestone 1, explore Fly-like managed postgres:

> There is no uncloud-specific solution for postgres backups yet, but any existing solution that works with Docker should work perfectly. […] While a generic docker volume backup is not the best approach for creating a backup for a DB, you can still consider using it: https://github.com/offen/docker-volume-backup
>
> After finalising the work on the first milestone […], I'm hoping to explore options of integrating DB deployment and management in uncloud. Ideally, providing developer experience similar to Fly's self-hosted postgres.

[Discussion #262](https://github.com/psviderski/uncloud/discussions/262), 2025-10-10.

**Self-healing by moving containers is intentionally avoided.** Redundancy idea is primary/standby containers, not volume failover. Databases as first-class in 2026 (`uc postgres` / Compose), inspired by Fly postgres-flex. [Discussion #223](https://github.com/psviderski/uncloud/discussions/223), 2026-02-01.

**Hub (paid, design preview) lists “backups and recovery” as Day-2 ops**, not cluster-native ZFS. Newsletter: hosted option helps with “observability, monitoring, alerts, backups and recovery.” Hub page ([uncloud.run/hub](https://uncloud.run/hub)) currently lists observability; “Where Hub goes next” is team access and push-to-deploy, not volumes.

**Volume model the author shipped** ([Issue #47](https://github.com/psviderski/uncloud/issues/47), 2025-04-08):

- Named volume without `external: true`: create on one suitable machine; services/replicas that share it schedule there.
- `external: true`: must already exist; schedule onto the machine that has it.
- Same name may exist on multiple machines; then any of those machines is eligible.

**Global vs replicated** ([Issue #234](https://github.com/psviderski/uncloud/issues/234), 2025-12-29): global services auto-provision an independent same-named volume on each targeted machine. Replicated services keep one-machine co-location. Shipped in [PR #243](https://github.com/psviderski/uncloud/pull/243) (merged 2026-02-02).

**NFS via the `local` driver is intended**, and other installed Docker volume drivers should work; scheduling semantics for non-local drivers are unsettled:

> In fact, other drivers should be supported as well. I was just not sure if the scheduling semantic should be different for them. For example, currently multiple replicas of the same service using a local volume are scheduled on the same machine to share the same volume because volumes are not automatically shared/replicated to other machines.

[Issue #210](https://github.com/psviderski/uncloud/issues/210#issuecomment-4086911476) (comment 2026-03-19). Docs updated to: volume drivers = `local` (NFS, CIFS/Samba) plus manually installed third-party drivers.

**HN, 2025-04-28:** “the volume management system […] provides the cluster semantics to the good old Docker volumes. It uses a constraint-based scheduler that ensures services sharing volumes are properly co-located.” [Ask HN comment](https://news.ycombinator.com/item?id=43818961).

**Do not auto-create a second empty volume when the machine that might hold the existing one is down.** Warn; user may still proceed to recover fresh. [Issue #120](https://github.com/psviderski/uncloud/issues/120), 2025-09-11 / 2025-09-12.

**Stable milestone** includes “persistent volumes” as a *current* core feature (Docker volumes), not a future ZFS feature. [Milestone 1](https://github.com/psviderski/uncloud/milestone/1).

**`design.md` does not mention storage, volumes, or ZFS.** Orchestration aim: “container scheduling can only be initiated by a user. […] they won't be moved to other machines automatically.” [misc/design.md](https://github.com/psviderski/uncloud/blob/main/misc/design.md).

### User asked

Nobody asked Uncloud for ZFS, snapshots as a CLI, or volume quotas.

What they did ask:

| User | Ask | Source | Date |
|---|---|---|---|
| **vladyalk** | Replicated-mode persistent volume with Compose `sync: true`; postgres 1-replica should restart on another machine if the first is down; maybe Uncloud-managed GlusterFS | [Issue #242](https://github.com/psviderski/uncloud/issues/242) | 2026-01-08 |
| **zombiehoffa** | Don’t put volume replication inside Uncloud (volumes can be tens–hundreds of GB); they already use CephFS bind-mounted into the VM. “isn't glusterfs dead?” | same issue | 2026-01-08 |
| **zombiehoffa** | Compose NFS `driver_opts` ignored by `uc deploy`; bulk data is NFS; blocker for trying Uncloud | [#210](https://github.com/psviderski/uncloud/issues/210), [#184](https://github.com/psviderski/uncloud/issues/184) | 2025-12-07/08 |
| **dasunsrule32** | Keep bind mounts to local disk or host NFS passthrough for both global and replicated apps; wanted an opt-out if auto-provisioning volumes would interfere | [#234](https://github.com/psviderski/uncloud/issues/234) | 2025-12-29 |
| **jreoka** | Easiest local backup of postgres on Uncloud; “some sort of integrated backup option would be useful” | [Discussion #262](https://github.com/psviderski/uncloud/discussions/262) (from [#138](https://github.com/psviderski/uncloud/issues/138)) | 2025-10-09 |
| **spiffytech** | `uc service rm` must not delete volumes; assumed deploy-created volumes were owned by the service | [#177](https://github.com/psviderski/uncloud/issues/177) | 2025-11-14 |
| **spiffytech** | First `uc deploy --recreate` with a named volume failed (“no machines available that satisfy all constraints”) | [#176](https://github.com/psviderski/uncloud/issues/176) | 2025-11-14 |
| **luislavena** | Misleading “failed to list volumes” warning when a node is down; if a volume exists, creating a new empty one on another node “might be perceived as data loss (Eg. a DB service)” | [#120](https://github.com/psviderski/uncloud/issues/120) | 2025-09-10 |
| **CharlieBytesX** | If node1 dies, move containers; postgres operator / leader election | [Discussion #223](https://github.com/psviderski/uncloud/discussions/223) | 2026-01-24 |
| **jackhalford** (HN, self-host) | “I don’t understand the storage story […] How are compose volumes replicated?” Homelab apps (nextcloud, plex, …) have sqlite/filesystem state and “no clear way for HA replication” | [HN #43287368](https://news.ycombinator.com/item?id=43287368) | 2025-03-07 |
| **bjesuiter** | Deploy bun + sqlite named volume; stuck restarting (logs were the ask) | [Discussion #157](https://github.com/psviderski/uncloud/discussions/157) | 2025-10-26 |
| **miekg** | Multi-tenant isolation on shared machines (not ZFS quotas) | [Discussion #268](https://github.com/psviderski/uncloud/discussions/268) | 2026-03-12 |

Homelab/self-host **in general** (not Uncloud): Docker-on-ZFS writeups want per-app datasets, `refquota`, snapshots, bind mounts instead of `/var/lib/docker/volumes`. Those posts do not mention Uncloud. Do not attribute them as Uncloud wants.

### Inferred from docs / code (current model)

Official docs + scheduler comments. Not a future ZFS plan.

**Named Docker volumes, bind mounts, tmpfs.** Drivers: `local` (including NFS/CIFS via `driver_opts`) and installed third-party plugins. Compose `placement` unsupported; use `x-machines`. [`storage_opt` unsupported](https://github.com/psviderski/uncloud/blob/main/website/docs/8-compose-file-reference/1-support-matrix.md).

**Volumes pin placement.** Scheduler:

> Volumes used by global services will be created on all eligible machines.
> Services that share a volume must be placed on the same machine where the volume is located.
> If the volume is located on multiple machines, services can be placed on any of them.
> If a volume already exists on a machine, it must be used instead of creating a new one.
> A missing volume must only be created on one machine.

[`pkg/client/deploy/scheduler/volume.go`](https://github.com/psviderski/uncloud/blob/main/pkg/client/deploy/scheduler/volume.go). Cannot share one volume between global and replicated services.

**CLI is machine-bound:** `uc volume create|ls|inspect|rm` with `--machine`. Create: `-d/--driver` (default `local`), `-o/--opt` driver options. Rm: if `--machine` omitted, remove that name from **all** machines. [CLI](https://uncloud.run/docs/cli-reference/uc_volume/). `uc rm` **preserves** named volumes ([`uc rm`](https://uncloud.run/docs/cli-reference/uc_rm/)).

**Deploy creates missing volumes on the target machine.** [Deploy an app](https://uncloud.run/docs/guides/deployments/deploy-app/) (“Creates any missing volumes on target machines”). Pin a DB with `x-machines`:

> Specify where to deploy stateful services and create their data volumes

[Deploy to specific machines](https://uncloud.run/docs/guides/deployments/deploy-specific-machines/).

**Single-replica + named volume → `stop-first` updates** so two writers don’t corrupt the volume. Multi-replica + volume does **not** auto-switch (concurrent access assumed desired). Bind/tmpfs don’t trigger the switch. [Rolling deployments](https://github.com/psviderski/uncloud/blob/main/website/docs/4-guides/1-deployments/4-rolling-deployments.md).

**README:** “Persistent storage: Run stateful services with Docker volumes managed across machines.” That means *scheduled across machines*, not replicated.

**Third-party Docker volume plugins are allowed** by the support matrix. A host-installed ZFS Docker plugin *could* be used as `driver:` today. That is Docker plugin support, not an Uncloud ZFS feature. No Uncloud example of it.

**Issue #184 still open:** “Document the Volumes concept and how to use volumes” (concept doc: no replication; how-to: create/rm, bind, nfs). There is still **no** official volumes concept page.

**`VolumeOptions` TODO** in [`pkg/api/volume.go`](https://github.com/psviderski/uncloud/blob/main/pkg/api/volume.go): “we may need [Driver and Labels] in the future if we add support for isolated container-scoped volumes.” Not ZFS.

---

## 4. Gaps (wanted, Uncloud does not have)

| Gap | Evidence | Uncloud today |
|---|---|---|
| **ZFS datasets / `zfs` CLI / compose `x-zfs`** | No issues, no code, no docs | Docker volumes only |
| **Volume snapshots** | Author “thinking about” (#242, newsletter); no issue, no CLI | None |
| **Volume quotas / `refquota`** | **Zero** Uncloud mentions | None. Compose `storage_opt` unsupported |
| **Incremental backups / send-recv / S3 replication of volumes** | Author examples (#242, newsletter); user asked for *postgres* backup (#262) | None. Author: use Docker backup tools / pg_dump sidecars |
| **Restore volume onto another machine after failure** | Author (#242); user postgres restart-elsewhere (#242); user data-loss fear (#120) | Volume dies with the machine. New volume elsewhere is empty |
| **Clone / move volume between machines** | Author “not sure yet” (HN 2025-03-07) | None |
| **Managed distributed storage (Gluster/Ceph/Longhorn)** | User #242; author rejected | DIY: mount on host, bind-mount |
| **Auto-reschedule container+data when a machine dies** | Users #223, #242; author: no multi-machine self-healing | Pre-place replicas; volumes stay put |
| **Integrated DB backup / Fly-like postgres** | Users #262, #223; author after Stable | Not shipped |
| **Official volumes concept docs** | Author #184 still open | CLI + support matrix + placement/rolling guides only |
| **NFS/`driver_opts`** | User #210 (was broken) | **Fixed** (#211). Docs still easy to misread as “local only” until 2026-03-19 |
| **Service rm deleting volumes** | User #177 assumed it would | Volumes preserved by design |
| **Safe warning when volume’s machine is down** | User #120 | Warning exists; creating a replacement empty volume is still possible |

---

## 5. What this does **not** support

**There is no Uncloud ZFS roadmap.** The only ZFS sentence is an *example* next to device mapper, inside “I’m thinking,” on a closed “not planned” distributed-storage issue. The newsletter item is the same idea without naming ZFS. Neither is a design, API, Compose field, pool model, or quota rule.

Do not read Ployz `DESIGN_MANAGED_ZFS.md` / `ManagedZfs` back into Uncloud. Uncloud did not specify:

- operator-provisioned pools, sparse vdevs, or `--from POOL`
- required `refquota` / `x-zfs: 10G`
- a type distinct from Docker volumes
- overcommit as a pool property
- privileged daemon ZFS RPCs

**Do not treat as Uncloud wants:**

- Homelab ZFS blog patterns (per-app datasets, `refquota`, sanoid/syncoid) unless someone asked Uncloud for them — they did not.
- Unofficial Mintlify pages (`psviderski-uncloud.mintlify.app`) that invent k8s-style `placement.constraints` Uncloud does not support.
- Discord (invite [discord.gg/eR35KQJhPu](https://discord.gg/eR35KQJhPu); author said features/roadmap are discussed there). Not fetchable here. One Discord link in [#120](https://github.com/psviderski/uncloud/issues/120) is about a warning showing IPv6, not ZFS.

**Author explicitly does not want (near term):** Uncloud-managed distributed/replicated filesystems; k8s-style PVs; auto-moving stateful containers; Uncloud becoming a storage cluster.

**Closest honest summary of the author’s storage *direction* (not a ZFS spec):** keep volumes **local**; help **recover** with snapshots + backups + optional replication-to-elsewhere; put **HA in the app/DB** (Fly-like postgres), not in a clustered filesystem.

```
today:  Docker volume on one machine = placement pin
author: still local + snapshots/backups/restore  (ZFS named once as e.g.)
users:  NFS, backups, “don’t lose the disk”, “restart postgres elsewhere”
not:    Uncloud ZFS product, quotas, send/recv, or a published ZFS design
```

---

## Search log (2026-08-15)

| Query | Result |
|---|---|
| GitHub code `zfs` in `psviderski/uncloud` and `org:psviderski` | 0 files |
| GitHub issues `zfs` in `psviderski/uncloud` | 0 (comment on #242 not indexed) |
| GitHub discussions titles/bodies: volume/storage/backup | #262 backups; no ZFS |
| `site:github.com/psviderski/uncloud zfs` | no ZFS docs/code; volume PRs #211/#243 |
| `site:uncloud.run` volumes | CLI + support matrix + x-machines; no ZFS; no volumes concept page |
| Discord | not fetchable |
| Author Substack 2026-01-24 | “modern persistent volumes with snapshots, incremental backups, and replication” — no ZFS |
| HN Show HN 2025-03-06 | author: no volume replication; maybe clone/move/backup commands |
| Uncloud recipes repo `zfs` | 0 |

Fetched: README, `misc/design.md`, issues 47, 48, 59, 120, 176, 177, 184, 197, 210, 212, 234, 242, PRs 211/243, discussions 157/223/262/268, support matrix, rolling/global/x-machines/deploy guides, volume CLI pages, milestone 1, Hub page, Substack, HN 43285730 / 43818961.
