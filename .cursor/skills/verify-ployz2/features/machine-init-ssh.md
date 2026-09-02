# Machine init (SSH)

`ployz machine init USER@HOST` founds a Cluster on a remote Linux Machine: install (unless `--no-install`), initialize `ployzd`, write a new local context, optionally reserve hosted DNS, optionally deploy the Ingress Proxy.

## Sub-features

- `init-ssh` destination is `[ssh://]user@host[:port]`. `--connect` is rejected (`machine init creates a new context; do not use --connect`).
- `init-context` `-c` / `--context` (default `default`) fails if that name already exists in `--ployz-config`.
- `init-flags` `--storage none|zfs`, `--no-dns`, `--no-ingress`, `--ingress-backend caddy|zentinel|envoy` (clap default `caddy`), `--network` default `10.210.0.0/16`, `--version` default this CLI (`PLOYZ_DAEMON_VERSION`; `nightly` rejected).
- `init-reset` `--yes` / `PLOYZ_AUTO_CONFIRM` auto-confirm reset of an already-initialized Machine.
- `init-success` prints `Switched context to '<name>'`, optional `Reserved Cluster domain: ...`, then `Initialised Machine <name> (<id>)`.

## How to get to it (user POV)

A user with SSH to a Linux Docker host runs `ployz machine init USER@HOST --storage none --no-dns`. `scripts/qualify-clean-init.sh VERSION USER@HOST...` is that recipe. `scripts/qualify-release.sh` installs the daemon from archives, then `machine init --no-install`.

## Driving it with helpers

Preconditions:

- Isolated `--ployz-config` from `helpers/prepare.sh`.
- A disposable `USER@HOST` that is not Nick's cluster. Using testkit/DinD only as that Linux Docker host, then this CLI command, is allowed. Testkit `initialize_first` is not this feature.

- **Shape.** `helpers/record.sh proof --cwd "$PWD" -- cargo test --locked --package ployz --test cli_shape -- clap_tree_matches_all_frozen_command_pages_and_declared_deviations --exact`. TSV rung 3.
- **Found.** `helpers/drive.sh proof machine init USER@HOST --context "verify-$INSTANCE" --name "verify-1" --storage none --no-dns --yes`. `--no-install` only if that host already runs this build's `ployzd` (SSH runs `ployzd dial-stdio`, default socket `/run/ployz/ployz.sock` on the remote).
- **Proof.** Stdout contains `Switched context to 'verify-<instance>'` and `Initialised Machine verify-1 (`. `ctx ls` lists that context. `machine ls` shows one Up Machine. Then [cluster-deploy.md](cluster-deploy.md).
- **Skip.** No disposable SSH destination, or no nested Docker image that can be that destination. Record the attempted command and the unmet precondition.

## Gotchas

- Isolated `ployzd` on this VM is not an init target unless you SSH to it. `--connect unix://...` is rejected.
- After initialize, `ployzd` exits so systemd can restart it (`ployz.service`). A nohup daemon does not come back by itself; the CLI then reports the Machine did not become ready.
- Creating `ployz-wg` needs CAP_NET_ADMIN. An unprivileged process gets `WireGuard operation failed: Netlink error: Failed to create WireGuard interface`.
- `--wg-port` only accepts `51820`.
- Hosted DNS default `--dns-endpoint` is `https://dns.uncloud.run/v1`. `--no-dns` skips that call.
- `qualify-clean-init.sh` is the authority-rung remote recipe. Capture `ployz machine init` stdout in evidence as well.
- Two participating Machines cannot share one host OS (WireGuard `ployz-wg`, Docker network `ployz`, Corrosion `127.0.0.1:7571`).
