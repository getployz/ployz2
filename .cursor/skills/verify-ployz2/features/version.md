# Version

`ployz --version`, `ployz -V`, and `ployz version` print this CLI's package version to stdout and exit 0. They do not contact a daemon, read a context, or require Docker.

## Sub-features

- Root flags `--version` and `-V` print the bare version line (example from this checkout: `0.1.2-beta.29`).
- Subcommand `ployz version` prints the same bare line.
- `ployz version -o '{{.Version}}'` prints that version; `ployz version -o 'v={{.Version}}'` prefixes `v=`.
- `ployz version -o '{{.Nope}}'` fails with stderr containing `unusable output template`.

## How to get to it (user POV)

Install or build `ployz`, then run it with no cluster and no config. Help lists `version` under Commands and `-V, --version` under Options. `ployzd version` is the matching daemon-side print (same string when both binaries come from one workspace build).

## Driving it with drive.sh

```sh
helpers/launch.sh proof          # optional for this feature; needed so doctor has a daemon
helpers/doctor.sh proof
helpers/drive.sh proof --version
helpers/drive.sh proof -V
helpers/drive.sh proof version
helpers/drive.sh proof version -o '{{.Version}}'
```

Proof: each successful drive's `*-stdout.txt` is exactly `<version>\n` matching `helpers/doctor.sh` / `ployzd version`. The failing template drive exits non-zero and `*-stderr.txt` contains `unusable output template`. Config yaml is unchanged.

## Gotchas

- `ployz version` does not report the connected daemon's version. Machine daemon skew shows up later as `WARNING: N Machine(s) run daemon version(s) different from CLI <version>.` on `machine ls`.
- Do not treat `ployzd --version` as a flag; the daemon subcommand is `ployzd version`.
