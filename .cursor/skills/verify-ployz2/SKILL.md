---
name: verify-ployz2
description: Verify Ployz product paths by driving the `ployz` CLI in isolation. Use when proving installer, daemon install, machine init, deploy, volumes, exec/logs, ingress, DNS, ACME, or Compose normalize against `evidence/product-paths.tsv`. Never drive Nick's live cluster.
---

# Verify Ployz

The product is install, found a Machine, deploy Compose, observe (`ls` / `ps` / logs / exec), then ingress, volumes, and DNS. The CLI is `ployz`. The daemon is Linux-only `ployzd`. Cloud dashboard is a different repo. There is no web UI here.

Coverage contract: `evidence/product-paths.tsv`. Feature files in [features/README.md](features/README.md) follow that list. Do not invent a different top-5. `ployz ctx` and `ployz version` are config/build switches under Launch/Doctor, not product paths.

This skill does not replace the repo's testing rungs in `AGENTS.md`. After a behavior change, name the rung and the test in the TSV. Climb only when a lower rung cannot go red.

1. Fastest local check (`scripts/test-cli-installer.sh`, `scripts/test-daemon-lifecycle.sh`, crate unit)
2. Layer 1 semantic (`cargo test`, not ignored)
3. CLI shape (`ployz/tests/cli_shape.rs`, `*_cli.rs`)
4. Informing cluster (`#[ignore = "informing"]` listed in `scripts/run-layer3-tests.sh`, via `ployz-testkit` DinD)
5. Authority (`scripts/qualify-release.sh` against musl archives on real Machines)

A live `ployz` drive here is extra evidence. It is not rung 4 or 5.

## Isolate first

Never touch the user's cluster.

| Kind | User / system | This run |
| --- | --- | --- |
| Config | `~/.config/ployz/config.yaml` (`PLOYZ_CONFIG`, `--ployz-config`) | `$RUN_DIR/config.yaml` |
| Socket | `/run/ployz/ployz.sock` | `$RUN_DIR/run/ployz.sock` |
| Data | `/var/lib/ployz` | `$RUN_DIR/data` |
| Metrics | `127.0.0.1:51090` | `127.0.0.1:<ephemeral>` |

Refuse those user/system paths. Fast CI injects a hostile `PLOYZ_CONFIG`; helpers unset it and pass `--ployz-config`.

Two **uninitialized** daemons can run side by side (`helpers/launch.sh a` and `helpers/launch.sh b`). Two **participating** Machines cannot share one host OS. Founding a Cluster needs a disposable `USER@HOST` (`scripts/qualify-clean-init.sh`) or testkit DinD as that Linux Docker host, then `ployz machine init` / `ployz deploy`. Calling testkit `initialize_first` or creating containers from the testkit API is not Deploy.

## Launch

From the repo root:

```sh
.cursor/skills/verify-ployz2/helpers/prepare.sh proof
```

Completion: isolated empty config, `target/debug/ployz` and `ployzd` at the same version (`cargo build -p ployz -p ployzd --locked` if missing). Enough for installer scripts, CLI shape, Compose preflight, and the local-init stub.

When the recipe talks to a Machine API on this VM (uninitialized daemon):

```sh
.cursor/skills/verify-ployz2/helpers/launch.sh proof
.cursor/skills/verify-ployz2/helpers/doctor.sh proof
```

`launch.sh` starts:

```sh
target/debug/ployzd \
    --data-dir "$RUN_DIR/data" \
    --socket "$RUN_DIR/run/ployz.sock" \
    --metrics-address 127.0.0.1:<free-port> \
    --log-level info
```

**Ready:** unix socket accepts; `GET /metrics` contains `ployz_ployzd_build_info{version="<same as ployz --version>"} 1`; log contains `started` and `uninitialized`. Defaults: data `/var/lib/ployz`, socket `/run/ployz/ployz.sock`, metrics `127.0.0.1:51090`. Hidden `--machine-api-address` is testkit/DinD only.

Uninitialized start may print `WARNING: local Docker is unavailable` when this process cannot open `/var/run/docker.sock`. Corrosion and WireGuard do not start until the Machine participates.

`ployz version` / `--version` / `-V` print the package version with no daemon. `ployzd version` is the daemon subcommand (not `--version`). `ployz ctx` reads `--ployz-config` only. Neither founds a Cluster.

**Teardown:** `.cursor/skills/verify-ployz2/helpers/cleanup.sh proof` (SIGTERM the recorded pid only, if any). Evidence stays under `/opt/cursor/artifacts/verify-ployz2/<instance>/`.

## Doctor

```sh
.cursor/skills/verify-ployz2/helpers/doctor.sh proof
```

Read-only, launched daemon only. Passes when the recorded pid is alive, `/proc/<pid>/cmdline` contains this run's `--socket` and `--data-dir`, those paths are not the system defaults, `--ployz-config` is under the run dir, `ployz --version` equals `ployzd version` equals the metrics gauge, and the log shows an uninitialized start. Writes `doctor.txt`, `ployzd.log`, and `metrics.txt`.

CLI-only recipes skip doctor and still refuse user/system paths via `prepare.sh`.

## Drive

Harness order:

1. Repo helpers (`prepare.sh` / `launch.sh` / `doctor.sh` / `drive.sh` / `record.sh`).
2. The rung named in `evidence/product-paths.tsv` for that path (`scripts/test-cli-installer.sh`, `cargo test` of the listed test, `scripts/run-layer3-tests.sh`, `scripts/qualify-release.sh`).
3. Generic PTY/tmux only for prompts that need a terminal.

```sh
helpers/prepare.sh proof
helpers/drive.sh proof version
helpers/drive.sh proof machine init
helpers/record.sh proof --cwd "$PWD" -- scripts/test-cli-installer.sh
helpers/drive.sh proof deploy --yes -f .cursor/skills/verify-ployz2/fixtures/sleep.yaml
```

`drive.sh` always passes `--ployz-config` for this instance. `--connect-unix` adds `--connect unix://$SOCKET`. Connect spellings: `unix://<absolute-path>`, `tcp://<host>:<port>`, `[ssh://]user@host[:port]`. Rejects `ssh+go://` and `ssh+cli://`.

Confirm: `Proceed with deployment to <context>? [y/N] ` on Deploy; generic `Continue? [y/N] `. No TTY: `confirmation requires a terminal; pass --yes to continue`. `--yes` / `PLOYZ_AUTO_CONFIRM` skip ordinary confirmation only. They cannot confirm Data Loss names (`machine rm`, `project rm --volumes`).

`--skip-health` skips post-start health monitoring on Deploy commands. `--no-install` on `machine init`/`add` skips installing Docker and `ployzd` on the destination. `--check` on `ployz build` does not build images. `--no-dns` skips hosted-domain reserve; `--no-ingress` skips founding Ingress Proxy Deploy.

Recipes: [features/README.md](features/README.md).

## Evidence

Write under `/opt/cursor/artifacts/verify-ployz2/<instance>/` (`VERIFY_PLOYZ2_EVIDENCE` overrides the root). `drive.sh` / `record.sh` record cmd, stdout, stderr, exit. Doctor adds `doctor.txt`, `ployzd.log`, `metrics.txt`. Cleanup must not delete this directory.

Proof bar:

- Drive the user command (`ployz ...`, `curl` of `install.sh` as the CLI installer contract, or the TSV script/test). Do not call testkit `initialize_first`, hidden `--machine-api-address`, or an internal RPC client and label that Deploy / init.
- Capture the action and the resulting state.
- Mocks only at a production boundary (hosted DNS `https://dns.uncloud.run/v1`, Cloud `ployz.dev`, ACME). Do not stub `ployzd`.
- If a path cannot run, write the skip with the missing precondition (SSH host, nested Docker / testkit image, public hostname for ACME, Docker group on this process). Do not invent Cluster tables.

## Cleanup

```sh
.cursor/skills/verify-ployz2/helpers/cleanup.sh proof
```

SIGTERM the pid in `run.env` if present, then remove `/tmp/verify-ployz2/<instance>/`. Never `pkill` by name. Confirm evidence still exists.

## Helpers

All under `.cursor/skills/verify-ployz2/helpers/`:

```sh
helpers/prepare.sh <instance>
helpers/launch.sh <instance>
helpers/doctor.sh <instance>
helpers/drive.sh <instance> [--connect-unix] <ployz-args...>
helpers/record.sh <instance> [--cwd DIR] -- <command...>
helpers/cleanup.sh <instance>
```

`seed-contexts.sh` writes a fake `ctx` yaml for isolation checks. It is not a Cluster.

Run `cleanup.sh` after every failed iteration so sockets and metrics ports are not stranded.

## Proved vs skipped

Fill this from the live pass on this VM. Source of truth for recipes remains the feature files. Copy the table into [features/README.md](features/README.md) when it changes.

Pending the live pass on this rewrite.
