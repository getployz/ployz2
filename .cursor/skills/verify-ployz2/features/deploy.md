# Deploy

`ployz deploy` applies a Compose file as one bounded Deploy: plan against this observer's snapshot, confirm, execute, print what completed / failed / was not attempted. It is not a controller and does not keep reconciling after the command returns.

## Sub-features

- Inputs: `-f` / `--file` (default `compose.yaml`), optional service names, `-p` / `--project-name` (`COMPOSE_PROJECT_NAME`), `--profile`, `--no-build`, `--recreate`, `--skip-health`.
- Confirm prompt: `Proceed with deployment to <context>? [y/N] `. `--yes` / `PLOYZ_AUTO_CONFIRM` skips that prompt. Without a TTY and without `--yes`, confirmation fails with `confirmation requires a terminal; pass --yes to continue`.
- Compose preflight uses the client-side Docker Compose plugin (`docker compose config`). Missing Docker CLI/plugin yields a `ployz deploy`-specific prerequisite error naming `https://docs.docker.com/compose/install/`.
- Sister user commands that share the Deploy planner: `ployz run`, `ployz service run`, `ployz scale`, `ployz ingress deploy`. `--skip-health` is on those; `--recreate` is not on `scale`.
- Related observation commands after a Deploy: `ployz ls` / `service ls` (`SERVICE ID	SERVICE	CONTAINERS	HOOKS`), `ployz ps` (`CONTAINER ID	SERVICE	KIND	MACHINE	STATE`), `ployz project ls` (`PROJECT	SERVICES	VOLUMES`). Project listings also print `WARNING: Live Observation is observer-relative and not globally complete`.

## How to get to it (user POV)

User has a participating Cluster context (from `machine init`) and a Compose file in the working directory. They run `ployz deploy`, read the plan, type `y`, and watch per-operation results. Automation passes `-y`.

## Driving it with drive.sh

Compose preflight **without** a Cluster (honest skip of execution):

```sh
helpers/launch.sh proof
helpers/doctor.sh proof
# from a directory with compose.yaml, Docker Compose plugin present:
helpers/drive.sh proof deploy --yes
```

If Docker Compose is missing, proof is the prerequisite stderr (mentions `ployz deploy` and the Compose install URL), not a successful Deploy.

Full Deploy (skip without a participating Machine and a real Compose project):

```sh
helpers/drive.sh proof --connect-unix deploy --yes -f compose.yaml
```

Proof: transcript includes the plan and either the confirm prompt or skipped confirm via `--yes`; outcome lists completed operations and any per-target failures/omissions; `ployz ps` / `ls` afterwards show the new Service Containers. Partial success is a valid outcome — do not require all-or-nothing.

Do not call testkit container-create helpers and label that a Deploy.

## Gotchas

- `--skip-health` skips the monitoring period and health checks after new containers start. It does not skip planning or execution.
- `--yes` does not confirm Data Loss names on remove paths (`machine rm`, `project rm --volumes`).
- Project names with underscores or uppercase are rejected (strict DNS-safe names).
- Isolated uninitialized `ployzd` cannot execute a Deploy; expect `Machine is not participating` if you `--connect-unix` without founding a Cluster.
