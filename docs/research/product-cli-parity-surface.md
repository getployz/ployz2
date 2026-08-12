# Uncloud product and CLI parity surface

## Question and baseline

What observable product, CLI, configuration, Compose, hosted-service, and test surface does a Rust reconstruction need to classify for parity?

This report uses `psviderski/uncloud` commit [`b7e224a1eff98813b1d1a32034d977be24be994e`](https://github.com/psviderski/uncloud/tree/b7e224a1eff98813b1d1a32034d977be24be994e) as the frozen baseline. It describes behavior rather than Go package boundaries. It does not decide which features Ployz2 will preserve or exclude.

## Executive findings

1. The product surface is wider than `init`, `add`, `run`, and `deploy`. The root CLI exposes machine, context, service, image, volume, Caddy, managed DNS, WireGuard inspection, logs, exec, proxy, and version workflows. Most service commands are available both at the root and under `uc service`. [`uc` registers this whole tree explicitly](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/main.go#L131-L161).
2. The stable product boundary is not the entire Compose specification. Uncloud parses Compose with `compose-go`, then implements a documented subset plus five service or project extensions and one secret extension. It intentionally ignores or rejects the rest. [The published support matrix is the best high-level inventory](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/8-compose-file-reference/1-support-matrix.md#L1-L92).
3. “Unsupported” has two observable meanings. Several unsupported service keys produce warnings and loading continues. Other cases, including relative bind mounts, external configs or secrets, bad secret drivers, conflicting port syntaxes, and port ranges, fail loading. [The loader prints validation results as warnings](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/project.go#L68-L85), while [the unsupported-feature test proves that warning behavior](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/project_test.go#L193-L345).
4. The managed DNS product is a client of existing Uncloud infrastructure, not a service embedded in the repository. The default endpoint is `https://dns.uncloud.run/v1`. The cluster stores the reservation endpoint, name, and token, verifies public Caddy instances, and submits wildcard A and AAAA records. [The CLI fixes the default endpoint](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/dns/reserve.go#L18-L69), and [the client describes reachability filtering and record construction](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/dns.go#L37-L145).
5. The repository has strong semantic oracles for Compose conversion, scheduling, deployment plans, container health, ports, secrets, configs, and multi-machine behavior. It has much weaker end-to-end coverage of the actual `uc` process, terminal rendering, exit status, prompts, and exact stdout or stderr. The e2e tests call Go clients directly rather than launching `uc`. [For example, Compose e2e loads a project and calls the deployment client](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/test/e2e/compose_deploy_test.go#L17-L73).

## Product workflows that form the parity surface

### Cluster bootstrap and membership

`uc machine init` provisions a remote machine over system SSH or the built-in SSH client, creates a new local context, initializes the cluster network, and by default reserves a managed domain and deploys Caddy. The default network is `10.210.0.0/16`, the default WireGuard listen port is `51820`, public IP detection defaults to `auto`, and Docker plus `uncloudd` installation can be skipped with `--no-install`. Caddy and DNS can be disabled independently. [The generated command reference records the arguments, defaults, and opt-outs](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_machine_init.md#L1-L65).

`uc machine add` performs the same remote provisioning for a new member, joins it to the existing mesh, deploys Caddy there unless disabled, and adds its connection as another local entry point. [Its command reference captures its transport and provisioning surface](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_machine_add.md#L1-L54). The documented product behavior is that adding a machine allocates a new machine subnet, exchanges WireGuard information, starts a local Corrosion replica, adds a fallback CLI connection, and allows any reachable machine to remain an entry point. [The README walks that behavior explicitly](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/README.md#L199-L248).

Two explicit bootstrap limitations are part of this baseline. Local initialization without a remote machine is not implemented and returns an error with a TODO in the source. [The local-init boundary is explicit](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/cli.go#L183-L190). Adding a machine redeploys Caddy to that machine even though the source notes that scaling would avoid possible small downtime. [That TODO and accepted downtime are recorded beside the operation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/machine/add.go#L181-L196).

`uc machine rm` removes membership and resets the remote machine by default. `--no-reset` deliberately leaves containers and data intact. [The removal command documents this destructive boundary](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_machine_rm.md#L1-L24).

Machine administration also includes:

| Command | Observable job | Command-specific flags |
|---|---|---|
| `uc machine ls` | List machines and their state, address, public IP, and WireGuard endpoints | `--output` |
| `uc machine rename OLD NEW` | Rename by machine name | none |
| `uc machine update MACHINE` | Change name, public IP, or advertised WireGuard endpoints | `--name`, `--public-ip`, `--wg-endpoint` |
| `uc machine rtt` | Show inter-machine round-trip time | none |
| `uc machine logs [SERVICE...]` | Merge system service logs, optionally filtered by machine and time | `--follow`, `--machine`, `--since`, `--tail`, `--until`, `--utc` |

These commands are registered as the complete machine subgroup, including aliases `machine|m`, `ls|list`, `logs|log`, and `rm|remove|delete`. [The command sources define the names and aliases](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/machine/root.go#L8-L24), and [the generated reference lists the public machine subgroup](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_machine.md#L1-L33).

### Local contexts and connection selection

The local YAML file defaults to `~/.config/uncloud/config.yaml`. It contains `current_context` and a map of named contexts. Each context is an ordered list of alternative connections. The CLI tries the connections in order until one works. A context is explicitly a local view of a cluster, not cluster state. [The configuration reference defines this model and its YAML](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/7-cli-config-reference.md#L1-L83), while [the connection loop establishes ordered failover behavior](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/cli.go#L94-L166).

One connection must use exactly one of these transports:

| Config key | Direct `--connect` form | Meaning |
|---|---|---|
| `ssh` | `[ssh://]user@host[:port]` | System SSH client, the default |
| `ssh_go` | `ssh+go://user@host[:port]` | Built-in Go SSH implementation |
| `tcp` | `tcp://host:port` | Direct gRPC over TCP |
| `unix` | `unix:///path/to/uncloud.sock` | Direct local Unix socket |

`ssh_cli` in config and `ssh+cli://` on the command line remain backward-compatible aliases for system SSH. [The command-line parser preserves the aliases](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/main.go#L53-L86), and [the config type validates exactly one transport](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/config/connection.go#L14-L72).

The context commands are `uc ctx` or `uc context`, `ctx ls|list`, `ctx show`, `ctx use [CONTEXT]`, and interactive `ctx connection|conn`. Invoking bare `uc ctx` is itself an interactive context switch. [The context root registers that dual behavior](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/context/root.go#L9-L26).

Config writes create parent directories with mode `0700` and the YAML file with mode `0600`. [The persistence method fixes those permissions](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/config/config.go#L54-L74).

### Service creation, deployment, and lifecycle

There are two service creation paths:

* `uc run IMAGE [COMMAND...]` constructs one service imperatively. It exposes image pull policy, replicated or global mode, replicas, placement by machine, environment, user, entrypoint, CPU and memory, shared memory, ulimits, privileged mode, volumes, published ports, and a custom service Caddyfile. [The command reference defines all flag formats and defaults](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_run.md#L1-L63).
* `uc deploy [SERVICE...]` loads one or more Compose files, optionally filters services and profiles, builds images unless disabled, presents a plan, and executes after confirmation. Its flags are `--build-arg`, `--build-pull`, `--file|-f`, `--no-build`, `--no-cache`, `--profile|-p`, `--recreate`, `--skip-health`, and `--yes|-y`. [The generated deployment reference is the observable contract](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_deploy.md#L1-L38).

`uc build [SERVICE...]` is independently observable. It supports checking configuration, including dependencies, selecting files and profiles, build args, no-cache and pull behavior, pushing images directly to selected cluster machines, or pushing to external registries. Cluster push and registry push are mutually exclusive. [The generated build reference lists `--check`, `--deps`, `--machine`, `--push`, and `--push-registry` among its flags](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_build.md#L1-L68), and [the constructor enforces the push conflict](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/build.go#L88-L100).

Service operations exist in both direct and grouped forms. For example, `uc logs` and `uc service logs` are the same command construction, not separate semantics. The direct commands are `exec`, `inspect`, `logs`, `ls`, `rm`, `run`, `scale`, `start`, and `stop`. The service group is aliased as `svc`. [The root registration reuses the same command constructors](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/main.go#L136-L160).

Service names are cluster-global. Compose project or stack prefixes are deliberately not added, so deploying two projects with the same service name addresses the same Uncloud service. [The deployment guide calls out this naming rule](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/4-guides/1-deployments/1-deploy-app.md#L316-L323).

| Command | Observable behavior and notable flags |
|---|---|
| `uc ls` | List services and published endpoints. Alias under group is `ls|list`. |
| `uc ps` | List every service container. Sort with `--sort service|machine|health`. |
| `uc inspect SERVICE` | Show detailed service and replica information. |
| `uc scale SERVICE REPLICAS` | Plan and apply a replica count change for replicated services. `--yes` bypasses confirmation. |
| `uc start SERVICE...` | Start all containers in one or more services. |
| `uc stop SERVICE...` | Stop service containers. `--signal` defaults to `SIGTERM`. `--timeout` defaults to ten seconds, and `-1` waits indefinitely. |
| `uc rm SERVICE...` | Remove services. Aliases under group are `rm|remove|delete`. |
| `uc exec [OPTIONS] SERVICE [COMMAND...]` | Select a random container by default or a specific full or prefix ID with `--container`. Supports detached execution and automatic TTY with `--detach` and `--no-tty`. Hidden `--interactive|-i` and `--tty|-t` flags remain for Docker CLI compatibility. Flags after `SERVICE` pass through to the container command. [The command reference specifies public selection and TTY behavior](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_exec.md#L1-L53), while [the constructor defines hidden compatibility flags and stops interspersed parsing](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/service/exec.go#L68-L87). |
| `uc logs [SERVICE[/CONTAINER]...]` | Merge logs across replicas and machines. With no service arguments, load service names from Compose. Supports `--file`, `--follow`, `--machine`, `--since`, `--tail`, `--until`, and `--utc`. [The reference defines selectors and accepted timestamp forms](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_logs.md#L1-L80). |
| `uc proxy SERVICE [LOCAL_PORT:]REMOTE_PORT` | Forward a service port to localhost until interrupted. It chooses the first running healthy replica and chooses a random local port when omitted. [The proxy reference states the selection behavior](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_proxy.md#L1-L27). |

Rolling deployment semantics are user-visible and belong to product parity. Updates replace containers one at a time. `start-first` is the default, but host-port conflicts and a single replica with a named volume switch to `stop-first` unless overridden. A failed new container stops the sequence. A prior container is restarted for `stop-first`, but already successful replacements and remaining old containers stay as they are. [The deployment guide specifies ordering and its safety exceptions](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/4-guides/1-deployments/4-rolling-deployments.md#L1-L65) and [specifies the deliberately partial rollback](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/4-guides/1-deployments/4-rolling-deployments.md#L155-L181).

Health monitoring defaults to five seconds and can end early when a configured Docker health check becomes healthy. `--skip-health` disables startup monitoring. Unhealthy containers after deployment leave Caddy routing but are not automatically restarted or rolled back by Uncloud. [The guide describes monitoring, the environment override, and the post-deployment non-action](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/4-guides/1-deployments/4-rolling-deployments.md#L67-L153).

Global mode is also imperative. A global service does not automatically gain a replica when a machine joins. The user reruns `uc deploy`. [The global-service guide makes that non-reconciliation boundary explicit](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/4-guides/1-deployments/3-deploy-global-services.md#L23-L32).

Each ordinary service container receives `UNCLOUD_MACHINE_ID`. Pre-deploy hook containers also receive `UNCLOUD_HOOK_PRE_DEPLOY=true`. [The Docker adapter injects the machine identity](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/server.go#L541-L550), and [the hook marker is injected with hook overrides](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/docker/server.go#L702-L707).

### Images and volumes

The image surface is:

* `uc image ls [REPO:[TAG]]` or the root alias `uc images [IMAGE]`, with repeatable or comma-separated `--machine` filters.
* `uc image push IMAGE`, which transfers a local Docker image to all or selected machines without requiring an external registry and optionally selects a multi-platform image with `--platform`.

The root alias is made by reusing the `image ls` command and changing only its use and examples. [The source makes that compatibility explicit](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/images.go#L1-L18), while [the push reference defines its machine and platform flags](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_image_push.md#L1-L50).

The volume surface is machine-local Docker volume administration:

| Command | Flags and behavior |
|---|---|
| `uc volume create VOLUME_NAME` | `--driver`, `--label`, required or selected `--machine`, and driver `--opt` |
| `uc volume ls|list` | Filter by `--machine`, or output names only with `--quiet` |
| `uc volume inspect VOLUME_NAME` | Search all machines or restrict with `--machine` |
| `uc volume rm|remove|delete VOLUME_NAME...` | Restrict machines, force in-use deletion, or bypass confirmation with `--machine`, `--force`, and `--yes` |

The generated volume references define these exact interfaces. [Create](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_volume_create.md#L1-L30), [inspect](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_volume_inspect.md#L1-L27), [list](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_volume_ls.md#L1-L28), and [remove](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_volume_rm.md#L1-L34).

### Ingress, Caddy, managed DNS, and WireGuard inspection

`uc caddy deploy` installs or upgrades a global `caddy` service across all or selected machines, optionally using a user image and prepending a global Caddyfile. It rolls existing instances. `uc caddy config` prints the generated config from the connected or selected machine. `uc caddy logs` reuses service log behavior for the Caddy service. [The deploy command reference records the product options](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_caddy_deploy.md#L1-L34).

When no image is given, Uncloud queries Docker Hub tags for the official `caddy` image, selects the greatest stable `2.x.x` tag, and falls back to `latest`. It runs Caddy globally with host ports TCP 80, TCP 443, and UDP 443 and persistent host directories. [That external lookup and generated service specification are explicit in the client](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/caddy.go#L15-L112).

Managed DNS commands are:

* `uc dns reserve [--endpoint URL]`, which reserves a cluster domain and updates records immediately if Caddy exists.
* `uc dns show`, which prints the current cluster domain.
* `uc dns release`, which removes the reservation from cluster state and reports it released. The hosted domain is not actually deleted because that API call remains a TODO.

The hosted interaction must remain compatible with the existing `https://dns.uncloud.run/v1` service if Ployz2 preserves this surface. Reservation uses `POST /domains`, while record changes use the stored bearer token. The cluster daemon, not merely the local command process, retains the endpoint and authentication material used for later record changes. The token is stored unencrypted with an explicit TODO to encrypt it. Before publishing records, the client probes `http://<public-ip>/.uncloud-verify`, accepts only responses containing that machine's ID, and publishes wildcard A and AAAA values only for reachable Caddy machines. [Reservation routing is in the command](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/dns/reserve.go#L42-L69), [the hosted HTTP operations are defined by the DNS client](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/dns/client.go#L41-L93), [reachability verification is in the client](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/dns.go#L147-L227), and [storage, the encryption TODO, and local-only release are explicit in cluster code](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/dns.go#L16-L24).

`uc wg show [-m MACHINE]` exposes the current WireGuard configuration for the connected or selected machine. It is inspection only. [The generated reference contains its complete surface](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_wg_show.md#L1-L31).

### Global CLI behavior

Commands generally inherit:

| Flag | Environment | Behavior |
|---|---|---|
| `--connect` | `UNCLOUD_CONNECT` | Ignore local config and connect directly with SSH, built-in SSH, TCP, or Unix socket |
| `--context`, `-c` | `UNCLOUD_CONTEXT` | Override the current local context |
| `--uncloud-config` | `UNCLOUD_CONFIG` | Override the YAML config path |

`uc machine init` is the important exception. Its local `--context|-c` names the newly created context, so it does not inherit the usual context override. [The init reference shows only `--connect` and `--uncloud-config` as inherited](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/9-cli-reference/uc_machine_init.md#L67-L74).

An explicit flag wins over its environment variable. [The binding helper only applies an environment value when the flag was not set](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/flags.go#L32-L39). If there is neither a config file nor `--connect`, but `/run/uncloud/uncloud.sock` exists, the CLI connects to that local socket automatically. [The root pre-run hook implements this fallback](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/main.go#L89-L104).

Other user-relevant environment inputs are:

| Variable | Observable effect |
|---|---|
| `UNCLOUD_AUTO_CONFIRM` | Supplies `--yes` for init, add, deploy, and scale |
| `UNCLOUD_DAEMON_VERSION` | Supplies `--version` for init and add |
| `UNCLOUD_HEALTH_MONITOR_PERIOD` | Changes the global deployment monitor default |
| `UNCLOUD_FAILED_CONTAINER_LOGS_TAIL` | Changes how many failed-container lines deploy prints |
| `UNCLOUD_SSH_CONTROL_PERSIST` | Changes system SSH connection sharing duration |
| `DEBUG` | Enables debug logging for `1`, `true`, or `yes` |
| `COMPOSE_FILE` | Supplies Compose paths through the standard Compose loader |
| `COMPOSE_DISABLE_ENV_FILE` | Disables automatic `.env` loading |

The first four are bound or read directly by command and API code. [Command bindings](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/deploy.go#L41-L78), [health default](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/container.go#L287-L301), and [failure log tail](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/deploy.go#L301-L325). [Debug parsing is limited to the three documented truthy spellings](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/log/env.go#L1-L18). The Compose loader uses OS environment first, `.env` next, and standard `COMPOSE_FILE` discovery. [Its option order is explicit](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/project.go#L21-L50).

`uc version` supports human output, JSON, or a Go template via `--output`. The generated docs command itself is hidden from normal help and is build tooling, not a product-management workflow. [The version command defines the output modes](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/version.go#L20-L60) and [the docs command marks itself hidden](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/docs.go#L27-L36).

## Compose compatibility surface

### Supported standard fields

The baseline documents these as supported:

| Area | Supported fields |
|---|---|
| Service process and image | `build`, `command`, `entrypoint`, `image`, `init`, `pull_policy`, `stdin_open`, `tty`, `user` |
| Environment and files | `configs`, `env_file`, `environment` |
| Runtime permissions | `cap_add`, `cap_drop`, `devices`, `gpus`, `pid` with `host` only, `privileged`, `sysctls`, `ulimits` |
| Resources | `cpus`, `mem_limit`, `mem_reservation`, `shm_size` |
| Health and shutdown | `healthcheck`, `stop_grace_period` |
| Logs | `logging`, defaulting to Docker's `local` driver |
| Storage | service `volumes`, named volumes, bind mounts, tmpfs, volume labels, external volumes, `local` and installed third-party drivers |
| Deploy | `mode` as `global` or `replicated`, `replicas` |
| Config objects | file-based and inline configs |

This inventory comes from the pinned [support matrix](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/8-compose-file-reference/1-support-matrix.md#L13-L84). It should be treated as the declared product surface, then checked against conversion tests before implementation.

### Limited standard fields

| Field | Exact limit |
|---|---|
| `depends_on` | Deployment ordering is supported. `service_completed_successfully` is not. Use `x-pre_deploy`. |
| `ports` | Only the Uncloud mapping is supported. Host publishing uses `mode: host`. HTTP and HTTPS use `x-ports`. Published port ranges are rejected. |
| `secrets` | Resolve into service environment values through `secret://name`. File mounts into containers are not supported. |
| `deploy.resources` | CPU, memory, and device reservations only. |
| `deploy.update_config` | `order` and `monitor` only. |

The support matrix declares these limits. [Service and deploy limits](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/8-compose-file-reference/1-support-matrix.md#L13-L58). The completed-service dependency is rejected because Uncloud models long-running independently managed services rather than jobs or coupled lifecycles. [The source comment records that design reason](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/service.go#L501-L512).

### Explicitly unsupported fields

| Area | Unsupported fields or forms |
|---|---|
| Services | `dns`, `dns_search`, `labels`, `links`, `mem_swappiness`, `memswap_limit`, custom `networks`, `security_opt`, standard service secret mounts, `storage_opt` |
| Deploy | `labels`, standard `placement`, `restart_policy`, `rollback_config` |
| Configs | External configs, short mount syntax |
| Secrets | External secrets, providers other than file, environment, or the `exec` driver used by `x-command` |
| Ports | A service cannot use both `ports` and `x-ports`. Published port ranges are rejected. |
| Bind mounts | Relative and home-relative sources are rejected. Use absolute paths. |

The broad declaration is in the [support matrix](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/8-compose-file-reference/1-support-matrix.md#L13-L84). The implementation warns for the first row and `service_completed_successfully` rather than failing project load. [The exact warning validator and its TODO are visible here](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/service.go#L455-L517). External configs and secrets, conflicting ports, port ranges, and relative binds are hard errors. [Config error](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/config.go#L20-L36), [secret errors](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/project.go#L147-L187), and [port errors](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/port.go#L17-L38).

This warning versus error split is part of observed compatibility. A rewrite should not infer from the matrix alone that every unsupported key fails.

### Uncloud extensions

| Extension | Scope and behavior |
|---|---|
| `x-context` | Top-level cluster context used by Compose-aware commands. `--context` and `--connect` take precedence. |
| `x-machines` | Service placement restricted to named machines. A single string and a list are accepted. |
| `x-ports` | String port syntax for Caddy HTTP or HTTPS ingress and direct host TCP or UDP publishing. |
| `x-caddy` | Inline custom service Caddyfile. |
| `x-pre_deploy` | One-off hook container using the service image, environment, volumes, placement, and resources. It must succeed before rollout. Default timeout is five minutes. |
| `secrets.<name>.x-command` | Run a local command and use stdout as a deployment-time secret value. |

The pinned [extension reference](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/8-compose-file-reference/2-extensions.md#L1-L125) defines the first five and command secret shorthand. The loader registers the service extensions directly. [Loader registration](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/project.go#L21-L45).

Secret resolution is local to the `uc deploy` process, happens after image building but before deployment planning, resolves a referenced value only once, and stores the resolved environment value unencrypted in distributed service state and Docker container configuration. [The secrets guide states the lifecycle and storage boundary](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/3-concepts/8-secrets.md#L10-L48) and [states the security consequences](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/3-concepts/8-secrets.md#L133-L141).

### Image naming and Compose discovery

Services with `build` and no tagged `image` receive Git-aware generated tags. The documented default includes project, service, commit date, seven-character SHA, and `.dirty` when applicable. A name without a tag gets the generated tag. A fully tagged image remains unchanged. Users may use Go template functions and Compose environment interpolation. [The pinned image-tag reference defines this observable transformation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/8-compose-file-reference/3-image-tag-template.md#L1-L50) and [its supported fields and interpolation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/8-compose-file-reference/3-image-tag-template.md#L52-L142).

Compose discovery follows `compose-go`: OS environment, optional `.env`, `COMPOSE_FILE`, then default Compose filenames in the current or parent directories. It removes the usual Compose project prefix from volume names. [The loader fixes that sequence and name transformation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/project.go#L21-L67).

## Existing hosted and platform interactions

Ployz2 can reuse these services or platforms without rebuilding them, but its compatibility decisions must account for their protocols and expectations:

| Dependency | Observable use |
|---|---|
| Uncloud DNS | Reserve cluster domains and manage wildcard A or AAAA records through `https://dns.uncloud.run/v1`. Current release forgets the reservation locally only. |
| Docker Hub | Pull application images and discover the latest stable official Caddy `2.x.x` image |
| Existing registries | Optional `uc build --push-registry` and normal image pulls |
| Unregistry protocol | Push local image layers directly to cluster machines without a registry, surfaced by `uc image push` and `uc build --push` |
| Git | Generate default and custom image tags from repository state |
| System SSH | Default remote provisioning and tunneled gRPC connection, including local SSH config support |
| Docker install service and Uncloud GitHub releases | The embedded provisioning script can install Docker and fetch a selected daemon release |
| Caddy and ACME | Caddy owns public routing, load balancing, certificate issuance, and renewal |
| Public IP discovery | Try `api.ipify.org`, then `ipinfo.io/ip`, then `ip-api.com` with a five-second request timeout |

The README declares direct image push, managed DNS, Caddy HTTPS, SSH management, and the machine provisioning flow. [Feature contract](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/README.md#L26-L48) and [provisioning transcript](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/README.md#L152-L197). The CLI actually embeds and streams its install script over SSH. [Provisioning source](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/machine.go#L31-L98).

The public-IP fallback order is source-visible and therefore reproducible without inventing a new discovery service. [The network helper lists all three providers](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/network/address.go#L72-L100).

The supported operating envelope is also explicit. The local CLI supports macOS and Linux. Windows requires WSL. Provisioned machines are tested on Ubuntu and Debian, need key-based SSH as root or a passwordless-sudo user, and support AMD64 or ARM64. Other Linux distributions may work but are not part of the tested boundary. [The install guide excludes native Windows](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/2-getting-started/1-install-cli.md#L1-L15), and [the demo prerequisites define the machine envelope](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/2-getting-started/2-deploy-demo-app.md#L19-L36).

One incompatibility risk needs a later decision: the current install script downloads `uncloudd` artifacts from the original Uncloud GitHub releases. Reusing that endpoint unchanged would install the Go daemon, not a Rust reconstruction. The product workflow should remain, but the artifact source cannot literally stay unchanged if Ployz2 owns its daemon. [The installer builds its release URL from the Uncloud repository](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/scripts/install.sh#L1-L25).

## Parity oracles in the baseline

### High-value semantic oracles

| Oracle | What it fixes |
|---|---|
| `website/docs/9-cli-reference/` | Command names, aliases, synopsis, flags, defaults, help, examples, and environment annotations generated from the command tree |
| `pkg/client/compose/*_test.go` | Compose parsing, extension conversion, unsupported warnings, secrets, configs, ports, Caddy, pre-deploy, placement, update config, resources, and image tag templates |
| `pkg/api/*_test.go` | Public validation and formatting for ports, services, containers, volumes, configs, and health |
| `pkg/client/deploy/*_test.go` | Update order, operation sequence, diffing, resource changes, volume scheduling, and pre-deploy placement |
| `test/e2e/compose_deploy_test.go` | Three-machine Compose deployment, redeploy, recreate, placement, volumes, and pre-deploy hooks |
| `test/e2e/service_test.go` | Global and replicated deployment, placement, volumes, health failures, lifecycle, logs, internal DNS, and metrics |
| `test/e2e/machine_test.go` | Rename and update validation, service continuity, public IP and endpoint changes, and removal cleanup |
| `test/e2e/exec_test.go` | Exit codes, stdout and stderr, detached execution, container selection, and error behavior |
| `test/e2e/compose_build_test.go` | Build and direct image push into a test cluster |
| `test/e2e/compose_configs_test.go` | Config creation, mount ownership, mode, and content |

The e2e suite clearly instantiates `ucind` clusters and then connects through the Go client. [Compose setup](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/test/e2e/compose_deploy_test.go#L17-L45), [service setup](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/test/e2e/service_test.go#L40-L112), and [exec assertions](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/test/e2e/exec_test.go#L52-L109).

Fixtures worth porting as black-box input data are:

* `test/e2e/fixtures/compose-basic.yaml`
* `compose-multi-service.yaml`
* `compose-placement.yaml`, `compose-placement-comma.yaml`, and `compose-placement-nonexistent.yaml`
* `compose-volumes.yaml` and `compose-global-volume.yaml`
* `compose-configs.yaml` plus `fixtures/configs/test-config.conf`
* `compose-predeploy.yaml`
* `compose-build-basic/`

They are consumed directly by the pinned e2e suite. [The basic fixture is loaded here](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/test/e2e/compose_deploy_test.go#L27-L46), and [the build fixture is used here](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/test/e2e/compose_build_test.go#L26-L92).

### Oracle limitations

The current e2e suite does not launch the `uc` executable. It validates lower-level client behavior, so it cannot by itself lock:

* exact exit statuses from Cobra command errors
* stdout versus stderr placement
* table formatting, color, progress rendering, and output stability
* TTY detection and cancellation behavior
* interactive prompt wording and defaults
* shell completion behavior
* root and subgroup aliases as invoked by a user
* config mutation across a full init, add, context switch, and remove session
* real access to the hosted DNS service
* real remote provisioning against the release and installer infrastructure

The direct CLI unit tests are narrow. They cover global flag extraction for completion, exec argument normalization, context argument validation, machine flag environment binding, config serialization, connection validation, install command construction, log selectors, table helpers, and a few display collectors. [Global completion flags are tested here](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/main_test.go#L1-L103), and [config persistence is tested here](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/cli/config/config_test.go#L1-L64).

The Rust project should therefore use two layers of parity tests:

1. Port reusable fixture and semantic cases from Go tests for domain behavior.
2. Add golden or structured black-box tests that invoke both pinned `uc` and the Rust CLI for the commands chosen for parity. Normalize nondeterministic IDs, times, progress frames, and terminal styling rather than weakening semantic assertions.

## Boundaries and contradictions to classify later

These are facts surfaced by the inventory, not recommendations:

1. The support matrix labels `ports` as host-mode-only and points HTTP or HTTPS users to `x-ports`, but the parser converts standard ports into Uncloud port specs and preserves Compose's default `ingress` mode. Tests should determine the intended compatibility for each short and long syntax before reimplementation. [Declared matrix rule](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/8-compose-file-reference/1-support-matrix.md#L31-L31) and [actual converter](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/port.go#L81-L145).
2. Service-level `secrets` is called limited in the matrix because `secret://name` environment references work, but the standard Compose `services.*.secrets` mount field itself generates an unsupported warning. This distinction must survive. [Warning validator](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/service.go#L486-L488) and [documented environment mechanism](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/website/docs/3-concepts/8-secrets.md#L10-L48).
3. Unsupported-key validation contains an explicit TODO to check more common unsupported features. Unknown or unsupported Compose data may therefore be accepted and ignored today. That TODO is itself part of the source baseline and should be carried into the Rust code as requested, rather than “completed” by broad strict validation. [The exact TODO is in the validator](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/client/compose/service.go#L455-L464).
4. `uc dns release` currently deletes only the cluster's stored domain record. It does not call the hosted service to release the name. The endpoint call is a source TODO and must not be silently invented during parity work. [The behavior and TODO are adjacent](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/internal/machine/cluster/dns.go#L97-L112).
5. Adding a machine may briefly disrupt Caddy because the current workflow deploys rather than scales. This is preserved as a source TODO, not permission for a new reconciler. [The source records the limitation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/cmd/uc/machine/add.go#L190-L196).
6. The generated CLI reference is comprehensive for command shape, but it is not a guarantee that every command has full e2e coverage. Command parity and domain parity should be tracked separately.
7. Reusing hosted Uncloud DNS is compatible with the stated scope. Reusing original release download URLs for daemon installation is not compatible with actually deploying the Rust daemon and needs a targeted product decision.

## Verification status

The report was derived from the frozen source tree and its first-party generated documentation. A focused Go test run was attempted for the CLI, API, Compose, and deployment packages. It could not start because the baseline declares Go `1.26`, while the installed Go toolchain reported that the `go1.26` toolchain was unavailable. No test failure was observed because compilation never began.
