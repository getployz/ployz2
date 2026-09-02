# Machine init (local)

Omitting the destination on `ployz machine init` is a documented stub. It does not found a Machine on this host. CLI_DEVIATIONS `local-machine-init-stub` keeps the positional optional so that path reaches this handler. That is a gap, not a proved product path.

## Sub-features

- `init-omitted` `ployz machine init` with no destination exits non-zero and prints `local machine initialisation is not implemented; specify a remote machine`.
- `init-connect-rejected` `ployz machine init --connect unix://...` is still invalid (`machine init creates a new context; do not use --connect`) when a destination is present; without a destination the stub wins first.
- `init-config-unchanged` the isolated `--ployz-config` is not written.

## How to get to it (user POV)

A user who wants a local Cluster runs `ployz machine init` with no `USER@HOST`. They get the not-implemented message and are told to specify a remote machine. Local founding is also what `ployz cloud enroll` does on this host (root installer), which is a different command.

## Driving it with helpers

Preconditions:

- `helpers/prepare.sh proof` (or `launch.sh` if you also want a daemon). No SSH.

- **Stub.** `helpers/drive.sh proof machine init`. Non-zero exit. `*-stderr.txt` is `local machine initialisation is not implemented; specify a remote machine`. `*-config-after.yaml` matches before.
- **Proof of the gap.** That transcript. Do not call this a founded Machine or a Cluster.
- **Deviation check.** `CLI_DEVIATIONS.md` still lists `local-machine-init-stub`. cli_shape requires that name. If the handler starts founding locally, this file is wrong and the TSV needs a row.

## Gotchas

- `unix:///path/to.sock` as a destination string is a Connection URI, not the omitted-destination stub. Users found Clusters with `USER@HOST`. Do not treat a unix destination as shipped local init.
- `provision_local` exists for Cloud enroll and requires root. It is not `ployz machine init` with no args.
- Success of [machine-init-ssh.md](machine-init-ssh.md) does not close this gap.
