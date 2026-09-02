# Compose normalize

`ployz deploy` and `ployz build` run the client-side Docker Compose plugin (`docker compose config`) before talking to a Machine. Project inputs stay relative to the Compose file. This does not require a Cluster or a Docker daemon. It does require the Docker CLI and Compose plugin.

## Sub-features

- `normalize-config` `docker compose --all-resources config --no-consistency --no-normalize --no-path-resolution --format yaml` then Ployz interprets the result.
- `normalize-plugin` missing Docker CLI or Compose plugin yields a command-specific prerequisite error naming `ployz deploy` (or `build` / bare `logs`) and `https://docs.docker.com/compose/install/`.
- `normalize-diagnostic` a failed `docker compose config` diagnostic is preserved when the version probe succeeds.
- `normalize-scope` only `deploy`, `build`, and bare `logs` probe Compose. Explicit-service `logs` does not.
- `normalize-relative` bind mounts and includes stay relative; working directory follows the Compose file even if you launched from a nested dir.

## How to get to it (user POV)

User runs `ployz deploy` in a project directory. Preflight either prints a Compose diagnostic, a plugin-install error, or continues into the Deploy plan.

## Driving it with helpers

Preconditions:

- `helpers/prepare.sh proof`. `/usr/bin/docker` plus the Compose plugin (this VM has them even when the daemon socket is not writable by `ubuntu`).

- **Valid file, no Cluster.** `helpers/drive.sh proof deploy --yes -f .cursor/skills/verify-ployz2/fixtures/sleep.yaml`. Compose must succeed. Then connect fails (no context / no participating Machine). That is normalize proof, not Deploy proof.
- **Invalid file.** `helpers/drive.sh proof deploy --yes -f .cursor/skills/verify-ployz2/fixtures/invalid.yaml`. Non-zero. Stderr is the Compose diagnostic, not a Machine RPC error.
- **Rungs.** Layer 1: `ployz/tests/compose.rs::normalized_surface_reaches_requested_specs`. CLI: `ployz/tests/compose_cli.rs::real_compose_normalizes_without_a_daemon_and_project_inputs_stay_relative` (skips if `/usr/bin/docker` is missing). Record: `helpers/record.sh proof --cwd "$PWD" -- cargo test --locked --package ployz --test compose_cli -- real_compose_normalizes_without_a_daemon_and_project_inputs_stay_relative --exact`.
- **Skip.** No `/usr/bin/docker`. Then drive a missing-plugin error by putting a fake `docker` earlier on PATH only if you must; prefer the skip.

## Gotchas

- `DOCKER_HOST` pointing at a dead daemon still allows `docker compose config`. The rung-3 test sets `DOCKER_HOST=tcp://127.0.0.1:1`.
- `ployz build --check` also loads Compose and does not build images.
- Success here is not [cluster-deploy.md](cluster-deploy.md).
