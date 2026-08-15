# Operator opts into the Machine Pool

A Machine Pool is operator-provisioned, never auto-created. `machine init` and `machine add` create it only when the operator explicitly opts in (`--storage` today; a TTY prompt can ask later). `--yes` with no `--storage` means no pool. On Machines that opted in, Docker's data-root is placed on the pool before `dockerd` first starts. Machines that skipped it keep an unbounded Docker data-root; `storage pool create` later is a stop-and-copy.

Default-on at init was rejected: unlike Caddy, a pool consumes real disk the moment it exists. A Managed Volume is a Docker Volume on the pool, not a second volume type. The operator picks the filesystem tool at init (`zfs` or `ext4`; `btrfs` later). Compose declares a bound, not a filesystem.
