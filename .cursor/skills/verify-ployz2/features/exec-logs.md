# Exec and logs

`ployz exec`, `ployz logs`, and `ployz machine logs` attach to running Service Containers on a Cluster. They need a participating Machine and at least one container (usually after [cluster-deploy.md](cluster-deploy.md)).

## Sub-features

- `exec-service` `ployz exec SERVICE COMMAND...` (`--container`, `-d`/`--detach`, `-T`/`--no-tty`). Also `ployz service exec`.
- `logs-service` `ployz logs` / `service logs` (`-f`/`--follow`, `-m`/`--machine`, `--since`, `-n`/`--tail` default `100`, `--until`, `--utc`). Bare `logs` and `deploy`/`build` probe Compose; explicit-service `logs` does not.
- `logs-machine` `ployz machine logs` (same log flags, no `--file`).
- `logs-ingress` `ployz ingress logs`.
- `exec-target` container as `service:container` or `project/service:container`. `/` is Qualified Service identity.

## How to get to it (user POV)

After `ployz deploy`, the user runs `ployz logs web`, `ployz exec web -- wget -qO- http://...`, or `ployz machine logs`.

## Driving it with helpers

Preconditions:

- Participating Cluster with a running Service Container.

- **Logs.** `helpers/drive.sh proof logs web -n 20`. Exit 0. Stdout has container log lines (the sleep fixture is quiet; prefer a service that prints).
- **Exec.** `helpers/drive.sh proof exec web -- echo verify-exec`. Stdout contains `verify-exec`. Use `-T` when there is no TTY.
- **Machine logs.** `helpers/drive.sh proof machine logs`.
- **Rung.** Informing: `ployz/tests/operator_cluster.rs::exec_service_logs_and_machine_logs_cross_a_real_two_machine_cluster` via `scripts/run-layer3-tests.sh`. TSV rung 5 is `gap`.
- **Skip.** No participating Machine or no running container. Do not exec into a testkit-created container and call that this feature unless `ployz deploy` created it.

## Gotchas

- Invalid `--since` fails before connecting (`invalid log time`).
- Bare `ployz logs` with `--file` default `compose.yaml` is Compose-scoped. `ployz logs web` is a service selector.
- `-f` on `logs` is `--follow`. `-f` on `deploy` is `--file`.
