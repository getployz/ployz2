# Cluster deploy

`ployz deploy` applies a Compose file as one bounded Deploy: plan against this observer's snapshot, confirm, execute, print what completed / failed / was not attempted. It does not keep reconciling after the command returns. Partial success is a valid outcome.

## Sub-features

- `deploy-compose` `-f` / `--file` (default `compose.yaml`), optional service names, `-p` / `--project-name` (`COMPOSE_PROJECT_NAME`), `--profile`, `--no-build`, `--recreate`, `--skip-health`.
- `deploy-confirm` prompt `Proceed with deployment to <context>? [y/N] `. `--yes` / `PLOYZ_AUTO_CONFIRM` skip it.
- `deploy-run` `ployz run` / `service run` (image + `--publish` / `-p`, `--name`, `--replicas`). Shares `--skip-health` and `--recreate`.
- `deploy-scale` `ployz scale SERVICE REPLICAS` / `service scale`. `--skip-health` yes; no `--recreate`.
- `deploy-ingress` `ployz ingress deploy` (founding Ingress Proxy). `--skip-health` and `--recreate`.
- `deploy-observe` after execution: `ployz ls` (`SERVICE ID	SERVICE	CONTAINERS	HOOKS`), `ployz ps` (`CONTAINER ID	SERVICE	KIND	MACHINE	STATE`).

## How to get to it (user POV)

User has a participating Cluster context from `machine init` and a Compose file. They run `ployz deploy`, read the plan, type `y`. Automation passes `-y`. Authority uses `scripts/qualify-release/compose.yaml`.

## Driving it with helpers

Preconditions:

- Isolated config.
- Full execution needs a participating Machine ([machine-init-ssh.md](machine-init-ssh.md) succeeded on a disposable host). Compose plugin is required before that ([compose-normalize.md](compose-normalize.md)).

- **Preflight without a Cluster.** `helpers/prepare.sh proof` then `helpers/drive.sh proof deploy --yes -f .cursor/skills/verify-ployz2/fixtures/sleep.yaml`. This is not a Deploy. Expect Compose to load, then a connect error (no context / no participating Machine). If the plugin is missing, stderr names `ployz deploy` and `https://docs.docker.com/compose/install/`.
- **Execute.** After a real init context: `helpers/drive.sh proof deploy --yes -f .cursor/skills/verify-ployz2/fixtures/sleep.yaml`. Then `helpers/drive.sh proof ps` and `helpers/drive.sh proof ls`.
- **Proof.** Transcript includes the plan and either the confirm prompt or `--yes`. Outcome lists completed operations and any per-target failures. `ps` / `ls` show the new Service Containers. Do not require all-or-nothing.
- **Rungs.** Layer 1: `ployz/tests/compose.rs::compose_plan_separates_volume_preview_from_service_operations`. Informing: `scripts/run-layer3-tests.sh` / `ployz/tests/deploy_cluster.rs::deploy_execution_preserves_partial_effects_and_never_repairs_them`. Authority: `scripts/qualify-release.sh`.
- **Skip.** No participating Machine. Do not create containers with testkit and call that this feature.

## Gotchas

- `--skip-health` skips the monitoring period after new containers start. It does not skip planning or execution.
- Project names with underscores or uppercase are rejected (strict DNS-safe names).
- Isolated uninitialized `ployzd` cannot execute a Deploy. `--connect-unix` then yields `Machine is not participating`.
- `ployz ingress deploy` is the Ingress Proxy. HTTP hostnames on app services are [ingress-http.md](ingress-http.md).
