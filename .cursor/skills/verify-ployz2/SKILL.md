---
name: verify-ployz2
description: Verify Ployz by driving the `ployz` CLI against an isolated Linux `ployzd`. Use when proving user-visible CLI behavior, machine/context/deploy/version flows, or capturing evidence from a disposable local daemon — not the Cloud dashboard repo.
---

# Verify Ployz

Ployz in this repo is the `ployz` CLI (Linux, macOS, Windows/WSL) talking to a Linux-only `ployzd`. Cloud dashboard is a different repo. There is no web UI or TUI here. Drive the CLI the way a user does.

This Linux VM can run `ployzd`. Cluster membership, Deploy, volumes, and ingress need a participating Machine (remote `machine init`, or Docker-in-Docker via `ployz-testkit`). An isolated `ployzd` starts **uninitialized**; that is enough for doctor, version, and contexts. It is not a Cluster.

## Isolate first

Never touch the user's cluster:

| Kind | User / system | This run |
| --- | --- | --- |
| Config | `~/.config/ployz/config.yaml` (`PLOYZ_CONFIG`, `--ployz-config`) | `$RUN_DIR/config.yaml` |
| Socket | `/run/ployz/ployz.sock` (`unix:///run/ployz/ployz.sock`) | `$RUN_DIR/run/ployz.sock` |
| Data | `/var/lib/ployz` | `$RUN_DIR/data` |
| Metrics | `127.0.0.1:51090` | `127.0.0.1:<ephemeral>` |

Refuse if a path is the user/system one. Fast CI injects a hostile `PLOYZ_CONFIG`; helpers unset it and pass `--ployz-config`.

Two **uninitialized** daemons can run side by side (`helpers/launch.sh a` and `helpers/launch.sh b`). Two **participating** Machines cannot share one host OS: Corrosion (`7571` TCP / `7570` UDP), WireGuard interface `ployz`, Docker network `ployz`, and Machine RPC `7569` collide. For a Cluster, use another VM/`USER@HOST`, or the informing testkit image — do not initialize Nick's machine.

## Launch

From the repo root (helpers are executable):

```sh
.cursor/skills/verify-ployz2/helpers/launch.sh proof
.cursor/skills/verify-ployz2/helpers/doctor.sh proof
```

`launch.sh` builds `cargo build -p ployz -p ployzd --locked` when `target/debug/ployz` or `target/debug/ployzd` is missing, then starts:

```sh
target/debug/ployzd \
  --data-dir "$RUN_DIR/data" \
  --socket "$RUN_DIR/run/ployz.sock" \
  --metrics-address 127.0.0.1:<free-port> \
  --log-level info
```

**Ready:** unix socket exists and accepts; `GET http://127.0.0.1:<port>/metrics` contains `ployz_ployzd_build_info{version="<same as ployz --version>"} 1`; log contains `started` and `uninitialized`. Defaults: data `/var/lib/ployz`, socket `/run/ployz/ployz.sock`, metrics `127.0.0.1:51090`. `--machine-api-address` is hidden (testkit/DinD only); do not use it on a user path.

Uninitialized start prints `WARNING: local Docker is unavailable: ...` on stderr when the process cannot talk to Docker. That is expected here if `/var/run/docker.sock` is not usable. Corrosion and WireGuard do not start until the Machine participates.

CLI-only commands (`version`, `completion`, `ctx` against a seeded config) do not need `ployzd` after the binaries exist. Still launch when the feature talks to a Machine.

**Teardown:** `.cursor/skills/verify-ployz2/helpers/cleanup.sh proof` (SIGTERM the recorded pid only). Evidence stays under `/opt/cursor/artifacts/verify-ployz2/<instance>/`.

## Doctor

```sh
.cursor/skills/verify-ployz2/helpers/doctor.sh proof
```

Read-only. Passes when the recorded pid is alive, `/proc/<pid>/cmdline` contains this run's `--socket` and `--data-dir`, those paths are not the system defaults, `--ployz-config` is under the run dir, `ployz --version` equals `ployzd version` equals the metrics gauge, and the log shows an uninitialized start. Writes `doctor.txt` into the evidence dir.

## Drive

Harness order:

1. Repo helpers (`launch.sh` / `doctor.sh` / `drive.sh` / `seed-contexts.sh`).
2. Existing CLI tests that spawn `CARGO_BIN_EXE_ployz` (`ployz/tests/context_cli.rs`, `version_cli.rs`, `cli_shape.rs`, `compose_cli.rs`).
3. Informing cluster tests (`#[ignore = "informing"]` in `scripts/run-layer3-tests.sh`) — they use `ployz-testkit` DinD, not a user session.
4. Generic PTY/tmux only for prompts that require a terminal.

```sh
.cursor/skills/verify-ployz2/helpers/drive.sh proof version
.cursor/skills/verify-ployz2/helpers/drive.sh proof --connect-unix machine ls
.cursor/skills/verify-ployz2/helpers/drive.sh proof ctx ls
```

`drive.sh` always passes `--ployz-config` for this instance. `--connect-unix` adds `--connect unix://$SOCKET` (direct, skips config). Connect spellings the CLI accepts: `unix://<absolute-path>`, `tcp://<host>:<port>`, `[ssh://]user@host[:port]`. It rejects `ssh+go://` and `ssh+cli://`.

Stable handles (clap names, not coordinates):

| User action | Handle | Observable |
| --- | --- | --- |
| Version | `ployz --version`, `ployz -V`, `ployz version` | stdout package version; `-o '{{.Version}}'` same; `-o '{{.Nope}}'` stderr `unusable output template` |
| Empty config | `ployz ctx ls` | stdout `No contexts found`, exit 0 |
| List contexts | `ployz ctx ls` | header `NAME	CURRENT	CONNECTIONS`; current marked `*` |
| Select context | `ployz ctx use <name>` | `Current context is now "<name>".`; config `current_context` |
| Show context | `ployz ctx show` | stdout is the current name |
| Default connection | `ployz ctx connection` | stdout `unix://...` (no TTY required) |
| Select connection | `ployz ctx connection unix://...` | `Default connection for context "<name>" is now "unix://...".` |
| Interactive select | `ployz ctx use` with no name | prompt `Select a context:`; without a TTY: `cannot Select a context interactively without a terminal` |
| Local init stub | `ployz machine init` (no destination) | stderr `local machine initialisation is not implemented; specify a remote machine` |
| Found a Cluster | `ployz machine init USER@HOST` | `Switched context to '<name>'` then `Initialised Machine <name> (<id>)` |
| List Machines | `ployz machine ls` | header `ID	NAME	MEMBERSHIP	STORAGE	SUBNET	GATEWAY	PUBLIC IP	ENDPOINTS	HOSTNAME	DAEMON	DOCKER	OS	KERNEL	ARCH` |
| Uninitialized list | `machine ls` via `--connect unix://...` | stderr `Machine RPC returned: Machine is not participating` |
| Deploy confirm | after a plan | `Proceed with deployment to <context>? [y/N] ` |
| Generic confirm | several commands | `Continue? [y/N] `; no TTY: `confirmation requires a terminal; pass --yes to continue` |

Interactive `ctx use` in tmux:

```sh
SESSION=verify-ctx-$$
tmux new-session -d -s "$SESSION" -- \
  env -u PLOYZ_CONFIG -u PLOYZ_CONNECT \
  "$PLOYZ_BIN" --ployz-config "$CONFIG" ctx use
# wait until capture contains "Select a context:"
tmux capture-pane -pt "$SESSION"
tmux send-keys -t "$SESSION" "1" Enter
tmux kill-session -t "$SESSION"
```

Feature recipes: [features/README.md](features/README.md).

## Evidence

Write under `/opt/cursor/artifacts/verify-ployz2/<instance>/` (`VERIFY_PLOYZ2_EVIDENCE` overrides the root). `drive.sh` records cmd, stdout, stderr, exit, and config before/after. Doctor writes `doctor.txt`. Cleanup must not delete this directory.

Proof bar:

- Drive the user command (`ployz ...`), not an internal RPC client, testkit `initialize_first`, or hidden `--machine-api-address`.
- Capture the action and the resulting state (stdout **and** config.yaml / docker / `machine ls`).
- Mocks only at a production boundary (hosted DNS `https://dns.uncloud.run/v1`, Cloud `ployz.dev`). Do not stub `ployzd`.
- `--yes` / `PLOYZ_AUTO_CONFIRM` skip ordinary confirmation only. They cannot confirm Data Loss names (`machine rm`, `project rm --volumes`).
- `--skip-health` skips the post-start health monitoring period on Deploy commands.
- `--no-install` on `machine init`/`add` skips installing Docker and `ployzd` (assumes they already run on the destination).
- `--check` on `ployz build` does not build images.
- `--no-dns` skips hosted-domain reserve; `--no-ingress` skips founding Ingress Proxy Deploy.

## Cleanup

```sh
.cursor/skills/verify-ployz2/helpers/cleanup.sh proof
```

Sends SIGTERM to the pid in `run.env`, waits, SIGKILL if needed, removes `/tmp/verify-ployz2/<instance>/`. Never `pkill` by name. After cleanup, confirm evidence still exists at `/opt/cursor/artifacts/verify-ployz2/<instance>/`.

## Helpers

All under `.cursor/skills/verify-ployz2/helpers/`:

```sh
helpers/launch.sh <instance>
helpers/doctor.sh <instance>
helpers/seed-contexts.sh <instance>
helpers/drive.sh <instance> [--connect-unix] <ployz-args...>
helpers/cleanup.sh <instance>
```

Run `cleanup.sh` after every failed iteration so sockets and metrics ports are not stranded.
