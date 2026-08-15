# Roast: Machine Pool and optional volume quotas

Read against the live tree at `cursor/managed-storage-grill-c4c8`. Every claim below cites either a
spec line or a file in the repo. Nothing here is a rewrite of the spec.

**The single worst finding first.** The spec's headline scenario — Sentry already exists, already
has data, and you add `x-quota: 10G` to bound it — is a silent no-op on `ployz deploy`. Not an
error. Nothing happens.

```
compose: volumes: {data: {x-quota: 10G}}
        |
        v
plan_deploy -> volume_operations()        ployz/src/deploy/planning.rs:120-166
        |
        +-- volume missing? -> CreateVolume  (quota could be applied here)
        |
        +-- volume EXISTS?  -> nothing emitted at all
                              -> deploy succeeds
                              -> quota never applied
                              -> operator believes Sentry is capped
```

`volume_constraints` (planning.rs:168-206) only pushes into `missing_volumes` when a volume has no
observed location. `x-quota` on an existing volume produces zero operations. Story 27 refuses when
there is *no pool*. Nothing refuses when there *is* a pool and the volume is already sitting
unbounded on the host root. That is the common case, and it is the one the Problem Statement is
written about.

---

## Vetos

### V1. Story 4 — `--yes` / no-TTY without `--storage` must NOT be a usage error

Reverse it. Under `--yes` or no TTY, a missing `--storage` means `none`, printed on one line.

Today `ployz machine init --yes ssh://host` works headless (`ployz/src/handlers/machine/init.rs:56`,
`helpers::confirm` short-circuits on `yes`). Story 4 breaks every one of those scripts on upgrade
day, and the spec never mentions it as a breaking change. The justification ("CI never guesses") is
backwards: `none` is not a guess, it is *literally the behaviour that machine already has*
(Story 10 defines `none` as "today's unbounded Docker"). Choosing it changes nothing and surprises
nobody.

Replacement rule: *"Without `--storage`, under `--yes` or no TTY, the Machine gets `none` and init
prints: `no Machine Pool (--storage none); named volumes are unbounded. Retrofitting a pool after
Docker exists costs a stop-and-copy.`"* That satisfies "no silent default" — it is not silent —
without breaking a single existing script.

### V2. Implementation Decision "Default size" — delete the default entirely

Reverse it. Picking `zfs` or `ext4` without `--storage-size` is a usage error naming the flag.

Work the formula on a real small VPS. 25G disk, 20G free:
`reserve = clamp(20% of 25G, 10G, 64G) = 10G`; `usable = 10G`; `pool = min(10G, max(100G, 2.5G)) = 10G`.

So the "conservative" default `fallocate`s **half the remaining disk** into an empty pool. And
because `--docker-on-store` is off by default (V-correct, Story 12), Docker images, container logs
and every non-quota'd volume are *still on the host root* — now with 10G instead of 20G. The
operator who ran `machine init --storage ext4` because they wanted Sentry to stop filling the disk
has just made the disk fill twice as fast, and the pool that caused it is empty.

There is no size that is safe to guess here, because the safe size depends on a second opt-in the
operator has not made yet. Make them type it.

### V3. Implementation Decision "Boot" — "Unmounted mountpoints are not world-writable so Docker cannot write underneath" is not a mitigation

This does not work and should be struck. `dockerd` runs as root. Permission bits do not restrain
root (`CAP_DAC_OVERRIDE`); `chmod 000` on the mountpoint stops nothing. If the oneshot loses the
race, Docker creates `volumes/data/_data` on the underlying root filesystem, the pool then mounts
over it, and the operator's data is invisible and unreferenced.

Replace with an ordering guarantee that systemd actually enforces: a generated `.mount` unit for the
pool plus `RequiresMountsFor=` on `docker.service` via a drop-in, and a sentinel file inside the pool
that the daemon checks before reporting a pool present. Note that `install.sh:19-22` sets
`"live-restore": true`, so a dockerd that came up on the wrong filesystem will keep containers
running on it.

### V4. Implementation Decision — "`x-quota` is a field on named volumes" inherits recreate semantics nobody asked for

Keep the field. Veto shipping it without an explicit exclusion, because as written it silently
does the wrong thing twice.

`compare_specs` (`ployz-core/src/domain/spec.rs:397-455`) contains
`!same_multiset(current_volumes, requested_volumes)`. `VolumeSource::Named` lives inside
`ServiceVolume`, which lives inside `volumes`. So adding a quota field means:

- Editing `x-quota: 10G` -> `20G` in compose returns `SpecChange::NeedsRecreate` and **restarts every
  container mounting that volume**, while (per the headline finding) applying no quota at all.
- That directly contradicts Story 32: "raising a quota to work live ... without restarting the
  container".

The same function already destructures `pre_deploy: _` and `update: _` — there is existing prior art
for "a spec field that must not trigger recreate". The spec must say so out loud:

> *Quota is excluded from `compare_specs`, like `pre_deploy` and `update`. A quota change produces an
> `EnsureVolumeQuota` operation, never a container replacement.*

And note the mirror problem: `volume_matches` (planning.rs:270-278) compares name + driver +
options. If quota joins that comparison instead, an existing 10G volume against a compose file
saying 20G makes the Machine ineligible and the operator gets `PlanError::NoEligibleMachines`, not a
quota error.

### V5. The `ployz storage` noun — four new commands, zero entries in the deviation ledger

Reverse. Fold everything into existing nouns:

| Spec | Replace with |
| --- | --- |
| `storage quota set data=20G -m m1` | `ployz volume quota data 20G -m m1` (or `volume create --quota`) |
| `storage ls` | quota/used columns on `ployz volume ls`; pool column on `ployz machine ls` |
| `storage pool create -m m1` | `ployz machine pool create m1` |
| `storage pool grow` | `ployz machine pool grow m1 <size>` |

`ployz volume` already has `create` / `inspect` / `ls` / `rm`, all with `--machine`
(`ployz/src/cli/mod.rs:424-459`), and `volume create` already takes `--driver` / `--opt` / `--label`.
`CLI_DEVIATIONS.md` is eleven lines, each one a rename or a single added subcommand, each with a
one-clause justification. A whole new top-level noun with a two-level subcommand tree does not fit
that ledger, and the spec never mentions the ledger exists.

Separately: `data=20G` as a positional is an argument grammar used nowhere in this CLI. `key=value`
here always arrives via a repeated flag (`--label`, `--opt`).

---

## Improvements

### I1. Add the missing preflight (this is the fix for the headline finding)

Add to Implementation Decisions, Preflight list:

> *Volume exists but is not on the Machine Pool -> refuse that volume, naming
> `ployz volume quota <name> <size> -m <machine>` and the fact that a stop-and-copy is required to
> move existing data onto the pool. Deploy never migrates volume data.*

Without this line, `x-quota` is decoration on every volume that already exists.

### I2. Story 17 and the formula disagree; pick one (or delete both per V2)

Story 17: *"all usable space up to 100G, a quarter of usable above that."*
Formula: `pool = min(usable, max(100G, 25% of usable))`.

At `usable = 200G`: story says 50G, formula says 100G. The formula only exceeds 100G once
`usable > 400G`, so between 100G and 400G it is a flat 100G, not a quarter of anything.

Worse, the *prose* version is non-monotonic: `usable = 100G` -> 100G pool; `usable = 101G` -> 25.25G
pool. A bigger disk gets a smaller pool. If any default survives V2, write the formula and delete
the prose, or write `pool = min(usable, max(100G, 25% of usable))` in the story and stop describing
it in English.

### I3. Story 32 is false on ext4; split it

Story 30 (quota a volume that already has data) + Story 32 (live, no restart) + the ext4 decision
("walk existing inodes, then `setquota`; **stop the container first**") cannot all hold. The
retroactive project-ID walk is exactly the Story 30 case.

Replacement for Story 32:

> *As an operator, I want raising an already-set quota to take effect live on both tools, and I want
> the first quota on an ext4 volume that already has data to tell me it needs a container stop before
> it does anything, so that "the fix for a full Sentry is one command" is true on ZFS and honestly
> two commands on ext4.*

### I4. Story 24 catches the wrong typo

`RawVolume` (`ployz/src/compose/model.rs:241-251`) has no `#[serde(flatten)] other` and no
`deny_unknown_fields`. A misspelled `x-qouta: 10G` on a top-level volume is silently dropped by
serde today — and cannot be caught generically, because Compose reserves `x-` for arbitrary
extensions. Story 24 only covers unknown keys *inside* the object, which is the typo nobody makes.

Two things to add:

1. State the limit in Further Notes: *"A misspelled `x-quota` key cannot be detected; Compose
   permits arbitrary `x-` keys on volumes. Deploy prints unrecognised `x-` keys on top-level volumes
   as a warning."*
2. Tighten Story 24: unknown keys **and a missing `size`** are errors. Follow the `x-caddy` shape too
   literally and you inherit `ployz/tests/compose.rs:343-348`, where `x-caddy: {}` silently means
   "absent" — so `x-quota: {}` would silently mean "unbounded", which is the exact failure mode this
   spec exists to prevent.

Also note `bytes_u64` (`ployz/src/compose/convert.rs:683-699`) accepts `b/k/m/g` only. `x-quota: 1T`
is an error, and bare `x-quota: 10` is **ten bytes**. Say which units are legal.

### I5. Say what a *failed* pool inspection means

The spec says "no pool -> refuse that volume". Pool inspection is a fan-out returning
`PartialResult`, and this repo is disciplined about that (`// UT-028: keep every target's success or
typed failure instead of warning and omitting it`, `ployz/src/cluster.rs:208`). A Machine whose
inspection RPC returned `Unavailable` is not a Machine with `--storage none`.

Add: *"A failed pool inspection is a Deploy failure for that Machine, not an absent pool. Only a
successful inspection reporting no pool refuses the volume."*

---

## Holes

### Load-bearing

**H1. Two projects, one volume, two quotas.** Story 29 blesses two projects both naming `data`
sharing one Docker Volume (and it is already true — `mounts.rs:103-113` builds `{project}_{key}`
then strips the `{project}_` prefix straight back off, so the Docker name is just `data`). Story 23
puts the quota in compose. Nothing says what happens when project A says `x-quota: 10G` and project
B says `20G`, or when B declares no quota at all. Given H1's parent finding, the answer today is
"first deploy wins, silently, forever". The spec needs a rule; "last writer wins" is at least
honest.

**H2. The bounded-Docker guard is keyed on the wrong thing.** Story 15 refuses `--no-install` plus
`--docker-on-store`, to stop init migrating a live data-root. But `install_docker()`
(`scripts/install.sh:73-84`) returns early with a warning when `dockerd` already exists — the flag is
not what determines whether Docker is live. So `machine init --storage ext4 --docker-on-store`
against a box that already runs Docker, *without* `--no-install`, walks straight into the migration
Story 15 forbids. Key the refusal on observed `dockerd`, not on `--no-install`.

**H3. Story 5 contradicts Story 14.** Story 5 lists "live Docker in the way" as a reason a *pool
pick* fails. Story 14 wants `--no-install` on a box with live Docker to *still create the pool* and
leave Docker alone. Live Docker blocks bounded Docker; it does not block a pool. Delete it from
Story 5's list.

**H4. The daemon cannot do what Story 37 asks.** `ployz.service` (`scripts/install.sh:139-168`) sets
`PrivateTmp=true`, `ProtectSystem=full`, `ProtectHome=read-only`, `ProtectKernelTunables=true`,
`ProtectControlGroups=true`. Any one of those implies `PrivateMounts=`, so ployzd runs in a private
mount namespace with propagation to the host turned off: a `zfs mount` or loop mount it performs is
invisible to Docker. `ProtectSystem=full` additionally makes `/etc` read-only, so it cannot write
the boot oneshot or touch `/etc/docker/daemon.json`. `storage pool create --machine m1` after the
fact (Story 37) needs both. The spec never mentions the unit file. Either the unit gains
`PrivateMounts=no` + a `ReadWritePaths=` for the pool and `/etc/systemd/system` (and says so), or
`pool create` is not a daemon RPC.

**H5. There is a third seam, not two.** "Deploy snapshot collects pool inspection only when the
project has at least one `x-quota` volume" requires the snapshot gatherer to know about the compose
project. `deploy_snapshot(&mut self, machines: Vec<MachineObservation>)`
(`ployz/src/cluster.rs:704-716`) is deliberately project-agnostic; `snapshot_from_partial` takes
exactly `(machines, containers, volumes)` and `ObservedDockerVolume` is `{id, driver, options}` —
it drops labels on purpose. Threading requested-spec knowledge into it, and widening
`ObservedDockerVolume`, is a real third seam the Testing Decisions section does not budget for.

### Smaller

**H6. Nobody says how three flags reach the installer.** `scripts/install.sh` is base64'd, piped to
`bash` over ssh, and configured **only** by environment variables (`ployz/src/provisioning.rs:37-48`)
— it takes no argv, and its stdin is the pipe carrying its own source, so it can never prompt. The
spec says "the installer sequences store creation" and stops. It should say: TTY picks happen
client-side before `provision()`; the choice arrives as `PLOYZ_STORAGE` / `PLOYZ_STORAGE_SIZE` /
`PLOYZ_DOCKER_ON_STORE`; pool creation runs **before** `install_docker()` in `main()`
(`install.sh:170-183`), because after it the data-root already exists.

**H7. `x-quota` on `external: true` is undefined.** Compose external means "I did not create this,
do not manage it". `mounts.rs:102-125` already special-cases external by dropping driver and labels.
The planner still emits `CreateVolume` for missing external volumes (`ployz/tests/compose.rs:418-422`
asserts exactly this). Say whether `x-quota` + `external: true` is an error or a bound.

**H8. Story 40 is unfalsifiable.** "A failed Deploy never destroys a volume." There is no
`RemoveVolume` variant in `DeployOperation` — the exhaustive match at `planning.rs:298-303` lists
every one, and volume removal is not among them. The spec cannot regress this because the capability
does not exist. It pads the story list and buys no test.

**H9. Story 29 is the same kind of padding.** It describes behaviour the prefix-strip in
`mounts.rs:103-113` already produces. As a *constraint on the quota work* it is load-bearing (see
H1); as a user story it reads as new work that is already done.

**H10. Enter-aborts fights the house style.** Story 2 wants Enter on the storage pick to abort. The
existing selection prompt (`ployz/src/handlers/context.rs:122-148`) treats empty input as "take the
default", and the Machine picker (`ployz/src/handlers/volume.rs:264-278`) treats empty as cancel.
Two conventions already exist; the spec should name which one it is copying, or the operator learns
that Enter means different things in different Ployz prompts.

---

## What it looks like against the Uncloud bar

Holds up: no `driver: zfs`, no compose key naming a filesystem, no cluster policy, no copied
answers, `zfs list` / `repquota -P` / `df` still tell the truth, `ensure` over `create`, no new
`VolumeSource` variant, no reconciliation.

Does not: a new top-level CLI noun with four commands (V5), a default that silently costs a small
VPS half its free space (V2), a `--yes` regression (V1), a documented mitigation that is not one
(V3), and a compose extension that succeeds while doing nothing on the one workflow the Problem
Statement describes (headline / I1).

Fix the headline finding and V1–V3 and this is a small, honest feature. Ship it as written and the
first operator to add `x-quota` to a running Sentry gets a green deploy and a full disk.
