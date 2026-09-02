# Installer

`curl ployz.sh` (repo `install.sh`) installs the `ployz` CLI binary for this OS/arch from GitHub releases. It does not install `ployzd`, systemd units, or found a Cluster.

## Sub-features

- `install-stable` resolves `latest`/`stable` via `https://ployz.sh/stable` then downloads `ployz_<os>_<arch>.tar.gz`.
- `install-beta` uses the `beta` channel file.
- `install-version` accepts an explicit version; `nightly` is rejected (`nightly is not a supported release channel`).
- `install-checksum` verifies SHA-256; a corrupt archive is not installed.

## How to get to it (user POV)

A user runs the published installer, typically `curl -fsSL https://ployz.sh | sh`, or `sh install.sh <version>` from this repo. `INSTALL_BIN_DIR` defaults to `/usr/local/bin`.

## Driving it with helpers

Preconditions:

- Repo root. Isolated `--ployz-config` is not required (this script does not read Ployz config).
- Do not run the live installer into `/usr/local/bin` on this VM. Drive the rung-1 contract instead.

- **Contract.** Run `helpers/record.sh proof --cwd "$PWD" -- scripts/test-cli-installer.sh`. Exit 0. Stdout ends with `CLI installer accepts valid checksums and rejects corrupt archives`.
- **Proof.** The transcript is the proof. That script fakes `curl`/`uname` and installs into a temp `INSTALL_BIN_DIR`. It is the TSV rung for `installer`.
- **Live curl.** Skip unless you have a disposable prefix (`INSTALL_BIN_DIR` under `/tmp/verify-ployz2/`). Never write Nick's `/usr/local/bin/ployz`.

## Gotchas

- Root `install.sh` is the CLI. `scripts/install.sh` is the Machine daemon installer ([daemon-install.md](daemon-install.md)).
- Channel URL default `https://ployz.sh`; GitHub URL default `https://github.com/getployz/ployz2`. The contract points both at `example.invalid`.
- This path does not start `ployzd`.
