# Feature map

Coverage contract: `evidence/product-paths.tsv`. The product-path files below are that list plus `machine-init-local` (CLI_DEVIATIONS `local-machine-init-stub`, still a stub). Do not replace them with ctx or version.

`cli-shape` is in the TSV at rung 3 (`ployz/tests/cli_shape.rs::clap_tree_matches_all_frozen_command_pages_and_declared_deviations`). It is a freeze test, not a user path. Run it with `helpers/record.sh`. It has no four-H2 file.

## Baseline

1. Binaries from this checkout (`target/debug/ployz`, `target/debug/ployzd`), same version string.
2. `helpers/prepare.sh <instance>` for CLI-only paths. `helpers/launch.sh <instance>` plus `helpers/doctor.sh` when the recipe needs an uninitialized daemon on this VM.
3. `--ployz-config` at the instance config. Never `~/.config/ployz/config.yaml`.
4. Ambient `PLOYZ_CONFIG` / `PLOYZ_CONNECT` / `PLOYZ_CONTEXT` unset (helpers do this).

## Driving conventions

- Prefer `helpers/drive.sh` for `ployz` and `helpers/record.sh` for TSV scripts and `cargo test`.
- Stable handles are clap command names, prompt strings, and table headers in each feature file.
- Interactive prompts need a TTY (tmux). Non-interactive: pass the argument, or `--yes` / `PLOYZ_AUTO_CONFIRM` where documented.
- Founding and Deploy against a Cluster use a disposable `USER@HOST` or testkit only as that Linux Docker host, then `ployz machine init` / `ployz deploy`. Testkit RPC container-create is not Deploy.

## Proof and skip

- **Proved:** user command or the TSV check, captured stdout/stderr/exit, and a side effect that matches the feature file.
- **Skipped:** name the missing precondition. Do not fake Cluster output from an uninitialized daemon.

## Feature entry contract

Each product-path file: one H1, one paragraph of user-visible behavior, then exactly these H2s in order: `Sub-features`, `How to get to it (user POV)`, `Driving it with helpers`, `Gotchas`.

## Product paths

| Path | File | TSV rungs |
| --- | --- | --- |
| installer | [installer.md](installer.md) | 1 `scripts/test-cli-installer.sh` |
| daemon-install | [daemon-install.md](daemon-install.md) | 1 `scripts/test-daemon-lifecycle.sh` |
| machine-init-ssh | [machine-init-ssh.md](machine-init-ssh.md) | 3 cli_shape; 5 `scripts/qualify-release.sh` (`scripts/qualify-clean-init.sh` is the remote recipe) |
| machine-init-local | [machine-init-local.md](machine-init-local.md) | not in TSV; deviation `local-machine-init-stub` |
| cluster-deploy | [cluster-deploy.md](cluster-deploy.md) | 2 compose plan; 4 `deploy_cluster.rs`; 5 qualify-release |
| named-volume | [named-volume.md](named-volume.md) | 2 compose volume options; 4 `volume_layer3.rs`; 5 qualify-release |
| exec-logs | [exec-logs.md](exec-logs.md) | 4 `operator_cluster.rs`; 5 gap |
| ingress-http | [ingress-http.md](ingress-http.md) | 4 `ingress_cluster.rs`; 5 gap |
| internal-dns | [internal-dns.md](internal-dns.md) | 4 `internal_dns_cluster.rs`; 5 gap |
| acme-certs | [acme-certs.md](acme-certs.md) | 4 `certificates_cluster.rs`; 5 gap |
| compose-normalize | [compose-normalize.md](compose-normalize.md) | 2 `compose.rs`; 3 `compose_cli.rs` |

Sisters of cluster-deploy (`run`, `scale`, `ingress deploy`) live in [cluster-deploy.md](cluster-deploy.md). Observation after Deploy (`ls`, `ps`) lives there too.

## Proof on this rewrite

Live pass 2026-09-02, instance `product`, CLI `0.1.2-beta.29`. Evidence: `/opt/cursor/artifacts/verify-ployz2/product/`.

| Path | Status |
| --- | --- |
| installer | **Proved.** `scripts/test-cli-installer.sh` exit 0. |
| daemon-install | **Proved.** `scripts/test-daemon-lifecycle.sh` exit 0 (passwordless sudo, faked PATH). |
| machine-init-ssh | **Proved with caveat.** User command `ployz machine init ubuntu@127.0.0.1:PORT` on isolated sshd. Initialize accepted `verify-1`. Unprivileged WireGuard netlink failed; root `ployzd` then participated. `machine ls` membership `up`. |
| machine-init-local | **Proved as gap.** Stub stderr `local machine initialisation is not implemented; specify a remote machine`. |
| cluster-deploy | **Proved.** `ployz deploy --yes --skip-health -f fixtures/sleep.yaml` created `verify/web`. `ps` Running, `ls` `1/1`. |
| named-volume | **Proved.** `volume create verify-data`; `volume ls` header `MACHINE	VOLUME	TYPE	QUOTA	USED	DRIVER`; Compose `verifyvol_verify-data`. |
| exec-logs | **Proved.** `exec -T web -- echo verify-exec`; `logs web -n 5` exit 0. |
| ingress-http | **Skipped.** No HTTP `x-ports` service. Informing: `ingress_cluster.rs` (testkit image not pulled). |
| internal-dns | **Skipped.** `resolv.conf` ExtServers `[10.210.0.1]`; `getent hosts data` failed. Informing: `internal_dns_cluster.rs`. |
| acme-certs | **Skipped.** No fake CA or public hostname. Informing: `certificates_cluster.rs`. |
| compose-normalize | **Proved.** `compose_cli.rs` test ok. `deploy --yes -f sleep.yaml` then `no contexts found in Ployz config`. Invalid yaml: Compose diagnostic. |
| cli-shape (TSV only) | **Proved.** `cli_shape.rs` exact test ok. |

## Rest of the user CLI

Index only. Full four-H2 treatment stays on the product paths.

| Command | What the user sees |
| --- | --- |
| `ployz machine add USER@HOST` | Join a remote Machine to this context. Same provisioning flags as init (`--no-install`, `--storage`, `--version`, `--wg-port` 51820 only). |
| `ployz machine rm MACHINE` | Remove. Data Loss names must be typed; `--yes` cannot confirm them. `--no-reset` if the Machine is unreachable. |
| `ployz machine inspect MACHINE` | Pretty JSON telemetry (Machine, Docker, bridge capacity). Deviation `machine-inspect-telemetry`. |
| `ployz machine ls` | Observation table `ID	NAME	MEMBERSHIP	STORAGE	SUBNET	GATEWAY	PUBLIC IP	ENDPOINTS	HOSTNAME	DAEMON	DOCKER	OS	KERNEL	ARCH`. `-o json`. Uninitialized daemon: `Machine RPC returned: Machine is not participating`. |
| `ployz machine rename` / `rtt` / `update` / `logs` | Rename, round-trip times, public IP / WG endpoints, Machine logs. |
| `ployz ls` / `service ls` | `SERVICE ID	SERVICE	CONTAINERS	HOOKS`. `-o json`. |
| `ployz ps` | `CONTAINER ID	SERVICE	KIND	MACHINE	STATE`. `--sort service\|machine\|health`. `-o json`. |
| `ployz inspect SERVICE` | Inspect a service. |
| `ployz project ls` / `project rm` | `PROJECT	SERVICES	VOLUMES`. `--volumes` on rm is Data Loss. Listings also print `WARNING: Live Observation is observer-relative and not globally complete`. |
| `ployz images` / `image ls` / `image push` | Image inventory and push. `-o json` on list. |
| `ployz build` | Build service images. `--check` does not build. `-p` is `--profile`. `--push` vs `--push-registry`. |
| `ployz start` / `stop` / `rm` | Start, stop (`--signal` SIGTERM, `--timeout` 10), remove services. |
| `ployz dns reserve` / `show` / `release` | Hosted cluster domain at `https://dns.uncloud.run/v1`. Not [internal-dns.md](internal-dns.md). |
| `ployz cloud enroll TOKEN` | Found or join through Cloud (`ployz.dev`). Installs local `ployzd` unless already present. |
| `ployz wg show` | WireGuard configuration. |
| `ployz proxy SERVICE PORT` | Proxy a local port to a service. |
| `ployz completion SHELL` | Native clap completion. |
| `ployz ctx` / `version` | Local config switch and package version. Supporting only. Empty `ctx ls`: `No contexts found`. |

Connect: `unix://<absolute>`, `tcp://host:port`, `[ssh://]user@host[:port]`.
