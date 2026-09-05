# Daemon install

`scripts/install.sh` installs `ployzd` onto a Linux Machine: binary, `ployz.service`, volume-plugin socket activation, then `systemctl restart ployz.service`. Users hit it through `ployz machine init` / `machine add` unless they pass `--no-install`.

## Sub-features

- `install-unit` writes `ployz.service` (`EnvironmentFile=-/etc/default/ployz`, `TimeoutStopSec=15`, `RestartPreventExitStatus=78`).
- `install-plugin-socket` writes `ployz-volume-plugin.socket` (`ListenStream=/run/docker/plugins/ployz.sock`) and enables the socket, not the service.
- `install-replace` restarts on missing unit or older daemon; equal latest and newer-than-requested daemons are left in place.
- `uninstall` is `scripts/uninstall.sh`: removes `ployzd` and Ployz state, not Docker's own data.

## How to get to it (user POV)

On a new Machine, `ployz machine init USER@HOST` SSHes and runs this installer (passwordless sudo required unless the SSH user is root). `--no-install` assumes Docker and `ployzd` already run. Authority installs from musl archives via `scripts/qualify-release.sh` (`PLOYZ_RELEASE_DIR` + `PLOYZ_VERSION`).

## Driving it with helpers

Preconditions:

- Passwordless `sudo` for the rung-1 script (it fakes `systemctl`/`docker` on PATH).
- Do not run `scripts/install.sh` against this VM's real `/usr/local/bin` or `/etc/systemd/system`.

- **Contract.** Run `helpers/record.sh proof --cwd "$PWD" -- scripts/test-daemon-lifecycle.sh`. Exit 0. Stdout ends with `daemon replacement and destructive uninstall contracts passed`.
- **Proof.** That script is TSV rung 1 for `daemon-install`. It uses a fake release tarball and a temp `INSTALL_BIN_DIR` / `INSTALL_SYSTEMD_DIR` / `PLOYZ_DATA_DIR`.
- **Real Machine.** Skip unless you have a disposable Linux host. Then the user path is [machine-init-ssh.md](machine-init-ssh.md) without `--no-install`.

## Gotchas

- Defaults: `PLOYZ_DATA_DIR=/var/lib/ployz`, `PLOYZ_RUN_DIR=/run/ployz`, `PLOYZ_USER=ployz`. Those are the paths isolation forbids on this VM.
- Volume plugin must be socket-activated before Docker is installed. The contract fails if that order flips.
- `PLOYZ_STORAGE=zfs` needs the installer; `--storage zfs` with `--no-install` errors `zfs storage preparation requires the installer; remove --no-install`.
