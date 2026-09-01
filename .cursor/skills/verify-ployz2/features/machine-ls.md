# Machine list

`ployz machine ls` (alias `list`) prints this entry Machine's Cluster Observation: each visible Machine's id, name, membership, storage, subnet, and runtime columns. It is observer-relative, not a global inventory. `-o json` / `--output json` prints the same objects as JSON.

## Sub-features

- Table header: `ID	NAME	MEMBERSHIP	STORAGE	SUBNET	GATEWAY	PUBLIC IP	ENDPOINTS	HOSTNAME	DAEMON	DOCKER	OS	KERNEL	ARCH`.
- JSON via `-o json`.
- If any row's daemon version differs from this CLI, stderr warns `WARNING: 1 Machine runs a daemon version different from CLI <version>.` (or `N Machines`).
- Related user paths: `ployz machine inspect MACHINE` (pretty JSON telemetry), `ployz machine rtt`, `ployz wg show`.

## How to get to it (user POV)

After `machine init` (or `ctx use` onto that context), the user runs `ployz machine ls`. Direct: `ployz --connect unix:///path/to/ployz.sock machine ls` or `tcp://127.0.0.1:7569` (Machine RPC port is `7569` on a participating Machine's management address, not the isolated metrics port).

## Driving it with drive.sh

Against the isolated uninitialized daemon (negative proof that listing needs participation):

```sh
helpers/launch.sh proof
helpers/doctor.sh proof
helpers/drive.sh proof --connect-unix machine ls
```

Proof: non-zero exit; `*-stderr.txt` contains `Machine RPC returned: Machine is not participating`. That is the user-visible uninitialized state, not a Cluster with zero rows.

Against a participating Cluster (skip unless `machine-init.md` remote path succeeded, or you attach `--connect` to a disposable testkit Machine API):

```sh
helpers/drive.sh proof machine ls
helpers/drive.sh proof machine ls -o json
```

Proof: header present; at least one row; membership `up` for the entry Machine; JSON parses and includes the same Machine id. Do not invent rows.

## Gotchas

- `drive.sh` without `--connect-unix` uses the isolated config. An empty config errors `no Ployz config or local daemon socket is available` instead of talking to the instance socket — pass `--connect-unix` or seed a context whose connection is that socket.
- Metrics (`127.0.0.1:<ephemeral>`) is Prometheus `GET /metrics`, not Machine RPC. Do not `--connect tcp://` that port.
- Listing is not Cluster truth; a second observer can disagree until replication catches up.
