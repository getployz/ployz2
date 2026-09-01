# Machine init

`ployz machine init [DESTINATION]` founds a Cluster on a **remote** Linux Machine: install (unless `--no-install`), initialize `ployzd`, write a new local context, optionally reserve hosted DNS, optionally deploy the Ingress Proxy. Omitting `DESTINATION` is a documented stub, not local founding.

## Sub-features

- Destination required for the real path: `[ssh://]user@host[:port]`. `--connect` is rejected (`machine init creates a new context; do not use --connect`).
- New context name `-c` / `--context` (default `default`); fails if that name already exists in `--ployz-config`.
- `--storage none|zfs`, `--no-dns`, `--no-ingress`, `--ingress-backend caddy|zentinel|envoy` (clap default `caddy`), `--network` default `10.210.0.0/16`, `--version` default this CLI's version (`PLOYZ_DAEMON_VERSION`; `nightly` rejected).
- `--yes` / `PLOYZ_AUTO_CONFIRM` auto-confirm reset of an already-initialized Machine. `--no-install` assumes Docker and `ployzd` already run on the destination.
- Success lines: `Switched context to '<name>'`, optional `Reserved Cluster domain: ...`, then `Initialised Machine <name> (<id>)`.

## How to get to it (user POV)

A user with SSH to a Linux Docker host runs `ployz machine init USER@HOST --storage none --no-dns` (see `scripts/qualify-clean-init.sh`). That is also how a context first appears for [contexts.md](contexts.md). Local-machine founding is not shipped: `ployz machine init` with no destination prints `local machine initialisation is not implemented; specify a remote machine`.

## Driving it with drive.sh

**Always capture the stub** (no SSH):

```sh
helpers/launch.sh proof
helpers/doctor.sh proof
helpers/drive.sh proof machine init
```

Proof: non-zero exit; `*-stderr.txt` is `local machine initialisation is not implemented; specify a remote machine`; config unchanged.

**Remote founding** (skip unless you have a disposable `USER@HOST` that is not Nick's cluster):

```sh
helpers/drive.sh proof machine init USER@HOST \
  --context "verify-$INSTANCE" \
  --name "verify-1" \
  --storage none \
  --no-dns \
  --yes
```

Proof: stdout contains `Switched context to 'verify-<instance>'` and `Initialised Machine verify-1 (`; `ctx ls` lists that context; `machine ls` against it shows one Up Machine. `--no-install` only if that host already runs this build's `ployzd`.

Do not initialize via testkit RPC (`initialize_first`) and call it this feature.

## Gotchas

- Isolated `ployzd` on this VM is not a valid init target. There is no local destination, and `--connect unix://...` is rejected on `machine init`.
- `--wg-port` only accepts `51820`.
- Hosted DNS default `--dns-endpoint` is `https://dns.uncloud.run/v1`. `--no-dns` skips that network call.
- `scripts/qualify-clean-init.sh` is the authority-rung remote recipe; it is not a substitute for capturing `ployz machine init` stdout in evidence.
