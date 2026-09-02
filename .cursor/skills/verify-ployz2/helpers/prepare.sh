#!/usr/bin/env bash
# Isolated Ployz config and binaries with no daemon.
# Usage: helpers/prepare.sh <instance>
# Completion: run.env and an empty --ployz-config exist; paths are not Nick's cluster.

set -euo pipefail
# shellcheck disable=SC1091
source "$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)/common.sh"

INSTANCE=${1:-}
verify_require_instance "$INSTANCE"
shift || true
[[ "$#" -eq 0 ]] || verify_die "prepare.sh takes only the instance name"

verify_binaries

RUN_DIR=$(verify_instance_dir "$INSTANCE")
EVIDENCE_DIR=$(verify_evidence_dir "$INSTANCE")
ENV_FILE=$(verify_run_env "$INSTANCE")

if [[ -f "$ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$ENV_FILE"
    if [[ -n "${PLOYZD_PID:-}" ]] && kill -0 "$PLOYZD_PID" 2>/dev/null; then
        verify_die "instance $INSTANCE is already running as pid $PLOYZD_PID; cleanup first"
    fi
    rm -rf "$RUN_DIR"
fi

DATA_DIR=$RUN_DIR/data
SOCKET_DIR=$RUN_DIR/run
SOCKET=$SOCKET_DIR/ployz.sock
CONFIG=$RUN_DIR/config.yaml
LOG=$RUN_DIR/ployzd.log

verify_refuse_shared "$SOCKET" "$DATA_DIR" "$CONFIG"

mkdir -p "$DATA_DIR" "$SOCKET_DIR" "$EVIDENCE_DIR"
: >"$CONFIG"
chmod 600 "$CONFIG"

unset PLOYZ_CONFIG PLOYZ_CONNECT PLOYZ_CONTEXT PLOYZ_AUTO_CONFIRM PLOYZ_DAEMON_VERSION || true

CLI_VERSION=$("$PLOYZ_BIN" --version)
DAEMON_VERSION=$("$PLOYZD_BIN" version)
[[ "$CLI_VERSION" == "$DAEMON_VERSION" ]] || verify_die "CLI $CLI_VERSION != daemon $DAEMON_VERSION"

cat >"$ENV_FILE" <<EOF
INSTANCE=$INSTANCE
PLOYZ_BIN=$PLOYZ_BIN
PLOYZD_BIN=$PLOYZD_BIN
DATA_DIR=$DATA_DIR
SOCKET=$SOCKET
CONFIG=$CONFIG
LOG=$LOG
RUN_DIR=$RUN_DIR
EVIDENCE_DIR=$EVIDENCE_DIR
CLI_VERSION=$CLI_VERSION
REPO_ROOT=$VERIFY_REPO_ROOT
EOF

printf 'prepared instance %s config %s evidence %s version %s\n' \
    "$INSTANCE" "$CONFIG" "$EVIDENCE_DIR" "$CLI_VERSION"
