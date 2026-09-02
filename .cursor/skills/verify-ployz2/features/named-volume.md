# Named volume

`ployz volume` creates, lists, inspects, and removes Docker Volumes on a Machine. Names are Machine-local. A Compose named volume is also created as part of [cluster-deploy.md](cluster-deploy.md).

## Sub-features

- `volume-create` `ployz volume create NAME` (`-d`/`--driver` default `local`, `-m`/`--machine`, `-l`/`--label`, `-o`/`--opt`). `--size` uses driver `ployz` (Provisioned Volume) and conflicts with `--driver`/`--opt`.
- `volume-ls` table `MACHINE	VOLUME	TYPE	QUOTA	USED	DRIVER`. `-q`, `-o json`, `-m`.
- `volume-inspect` JSON. Same name on two Machines is `ambiguous` until `-m` is set.
- `volume-rm` `--yes` / `PLOYZ_AUTO_CONFIRM`. Informing contract: `volume rm shared missing --machine machine-1 --yes` fails and leaves both `shared` volumes.
- `volume-compose` Compose `volumes:` top-level plus a service mount. Authority compose uses `qualify-data`.

## How to get to it (user POV)

After a Cluster exists, `ployz volume create shared -m machine-1` or `ployz deploy -f` a file with a named volume. Then `ployz volume ls`.

## Driving it with helpers

Preconditions:

- Participating Machine. Fixture `.cursor/skills/verify-ployz2/fixtures/named-volume.yaml`.

- **CLI create.** `helpers/drive.sh proof volume create verify-data`. Then `helpers/drive.sh proof volume ls`. Proof: ls header `MACHINE	VOLUME	TYPE	QUOTA	USED	DRIVER` and a `verify-data` row.
- **Compose.** `helpers/drive.sh proof deploy --yes -f .cursor/skills/verify-ployz2/fixtures/named-volume.yaml` then `helpers/drive.sh proof volume ls` contains `verify-data`.
- **Rungs.** Layer 1: `ployz/tests/compose.rs::one_named_volume_can_have_different_options_per_mount`. Informing: `ployz/tests/volume_layer3.rs::volume_cli_mounts_and_partial_results_stay_machine_local`. Authority: `scripts/qualify-release.sh` greps `qualify-data` in `volume ls`.
- **Skip.** No participating Machine. Uninitialized `--connect-unix volume ls` is `Machine is not participating`, not an empty table.

## Gotchas

- `--size` parse: positive integer plus `k`, `m`, `g`, or `t`.
- Volume identity is (Machine, name). Listing without `-m` fans out.
- `project rm --volumes` is Data Loss, not `volume rm --yes`.
