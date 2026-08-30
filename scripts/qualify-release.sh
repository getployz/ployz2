#!/usr/bin/env bash
# Qualify draft musl archives against real Linux Machines.
# Pass SSH targets. This script does not pick a cloud vendor.
#
#   PLOYZ_QUALIFY_HOSTS='root@192.0.2.10 root@192.0.2.11' \
#   PLOYZ_ARTIFACT_DIR=/path/to/dist \
#   scripts/qualify-release.sh
#
# Optional: PLOYZ_QUALIFY_DRY_RUN=1, PLOYZ_QUALIFY_SSH_OPTS, PLOYZ_QUALIFY_CONTEXT.

set -euo pipefail

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
COMPOSE_DIR=$ROOT/scripts/qualify-release
HOSTS=${PLOYZ_QUALIFY_HOSTS:-}
ARTIFACT_DIR=${PLOYZ_ARTIFACT_DIR:-}
DRY_RUN=${PLOYZ_QUALIFY_DRY_RUN:-0}
SSH_OPTS=${PLOYZ_QUALIFY_SSH_OPTS:-"-o StrictHostKeyChecking=accept-new"}
CONTEXT=${PLOYZ_QUALIFY_CONTEXT:-qualify}
CONFIG_DIR=

error() { echo "ERROR: $*" >&2; exit 1; }

need_archive() {
    [ -f "$ARTIFACT_DIR/$1" ] || error "PLOYZ_ARTIFACT_DIR is missing $1"
}

cli_archive() {
    case "$(uname -s):$(uname -m)" in
        Linux:x86_64) echo ployz_linux_amd64.tar.gz ;;
        Linux:aarch64) echo ployz_linux_arm64.tar.gz ;;
        Darwin:x86_64) echo ployz_macos_amd64.tar.gz ;;
        Darwin:arm64) echo ployz_macos_arm64.tar.gz ;;
        *) error "unsupported controller platform $(uname -s) $(uname -m)" ;;
    esac
}

daemon_archive() {
    case "$1" in
        x86_64) echo ployzd_linux_amd64.tar.gz ;;
        aarch64) echo ployzd_linux_arm64.tar.gz ;;
        *) error "unsupported Machine architecture $1" ;;
    esac
}

ssh_host() {
    # shellcheck disable=SC2086
    ssh $SSH_OPTS "$1" "${@:2}"
}

[ -n "$HOSTS" ] || error "set PLOYZ_QUALIFY_HOSTS to one or more user@host targets"
[ -n "$ARTIFACT_DIR" ] || error "set PLOYZ_ARTIFACT_DIR to the draft archive directory"
[ -f "$COMPOSE_DIR/compose.yaml" ] || error "missing $COMPOSE_DIR/compose.yaml"

read -r -a HOST_LIST <<<"$HOSTS"
[ "${#HOST_LIST[@]}" -ge 1 ] || error "PLOYZ_QUALIFY_HOSTS is empty"

need_archive ployz_linux_amd64.tar.gz
need_archive ployz_linux_arm64.tar.gz
need_archive ployzd_linux_amd64.tar.gz
need_archive ployzd_linux_arm64.tar.gz
need_archive "$(cli_archive)"

if [ "$DRY_RUN" != 0 ]; then
    echo "qualify dry-run"
    echo "hosts: ${HOST_LIST[*]}"
    echo "artifacts: $ARTIFACT_DIR"
    echo "compose: $COMPOSE_DIR/compose.yaml"
    echo "steps: install daemon from PLOYZ_RELEASE_DIR, machine init --no-install, machine add, deploy named volume qualify-data, volume ls"
    exit 0
fi

work=$(mktemp -d)
trap 'rm -rf "$work" "$CONFIG_DIR"' EXIT
tar -xzf "$ARTIFACT_DIR/$(cli_archive)" -C "$work"
PLOYZ=$work/ployz
[ -x "$PLOYZ" ] || error "CLI archive did not contain ployz"
version=$("$PLOYZ" version) || error "could not read ployz version from the artifact"
CONFIG_DIR=$(mktemp -d)
export PLOYZ_CONFIG=$CONFIG_DIR/config.yaml

install_host() {
    local host=$1 arch archive remote
    arch=$(ssh_host "$host" uname -m)
    archive=$(daemon_archive "$arch")
    remote=/tmp/ployz-qualify-$$
    ssh_host "$host" mkdir -p "$remote"
    # shellcheck disable=SC2086
    scp $SSH_OPTS "$ROOT/scripts/install.sh" "$ARTIFACT_DIR/$archive" "$host:$remote/"
    ssh_host "$host" sudo env \
        PLOYZ_RELEASE_DIR="$remote" \
        PLOYZ_VERSION="$version" \
        bash "$remote/install.sh"
}

for host in "${HOST_LIST[@]}"; do
    echo "install $host"
    install_host "$host"
done

first=${HOST_LIST[0]}
echo "machine init $first"
"$PLOYZ" machine init --yes --no-install --context "$CONTEXT" "$first"

i=1
while [ "$i" -lt "${#HOST_LIST[@]}" ]; do
    echo "machine add ${HOST_LIST[$i]}"
    "$PLOYZ" machine add --yes --no-install --context "$CONTEXT" "${HOST_LIST[$i]}"
    i=$((i + 1))
done

echo "deploy named volume"
"$PLOYZ" deploy --yes --context "$CONTEXT" -f "$COMPOSE_DIR/compose.yaml"

volumes=$("$PLOYZ" volume ls --context "$CONTEXT")
printf '%s\n' "$volumes"
printf '%s\n' "$volumes" | grep -Eq 'qualify-data' || error "named volume qualify-data did not appear in volume ls"

echo "qualify-release passed on ${HOST_LIST[*]}"
