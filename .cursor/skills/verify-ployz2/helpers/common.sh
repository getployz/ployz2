#!/usr/bin/env bash
# Shared paths and guards for verify-ployz2 helpers. Source this file; do not execute it.

set -euo pipefail

VERIFY_HELPERS_DIR=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
VERIFY_SKILL_DIR=$(CDPATH='' cd -- "$VERIFY_HELPERS_DIR/.." && pwd)
VERIFY_REPO_ROOT=$(CDPATH='' cd -- "$VERIFY_SKILL_DIR/../../.." && pwd)

VERIFY_HOME=${VERIFY_PLOYZ2_HOME:-/tmp/verify-ployz2}
VERIFY_EVIDENCE_ROOT=${VERIFY_PLOYZ2_EVIDENCE:-/opt/cursor/artifacts/verify-ployz2}

FORBIDDEN_SOCKET=/run/ployz/ployz.sock
FORBIDDEN_DATA_DIR=/var/lib/ployz
FORBIDDEN_CONFIG=${HOME:+$HOME/.config/ployz/config.yaml}
FORBIDDEN_CONFIG=${FORBIDDEN_CONFIG:-/root/.config/ployz/config.yaml}

verify_die() {
    echo "verify-ployz2: $*" >&2
    exit 1
}

verify_require_instance() {
    local name=${1:-}
    [[ "$name" =~ ^[A-Za-z0-9][A-Za-z0-9-]*$ ]] || verify_die "instance name must be alphanumeric or dash: ${name:-<empty>}"
}

verify_instance_dir() {
    printf '%s/%s\n' "$VERIFY_HOME" "$1"
}

verify_evidence_dir() {
    printf '%s/%s\n' "$VERIFY_EVIDENCE_ROOT" "$1"
}

verify_run_env() {
    printf '%s/run.env\n' "$(verify_instance_dir "$1")"
}

verify_load_run() {
    local env_file
    env_file=$(verify_run_env "$1")
    [[ -f "$env_file" ]] || verify_die "no run.env for instance $1 (launch first): $env_file"
    # shellcheck disable=SC1090
    set -a
    source "$env_file"
    set +a
    [[ -n "${INSTANCE:-}" && -n "${SOCKET:-}" && -n "${DATA_DIR:-}" && -n "${CONFIG:-}" ]] ||
        verify_die "run.env for $1 is incomplete"
}

verify_refuse_shared() {
    local socket=${1:-} data_dir=${2:-} config=${3:-}
    [[ "$socket" == "$FORBIDDEN_SOCKET" ]] && verify_die "refusing shared daemon socket $FORBIDDEN_SOCKET"
    [[ "$data_dir" == "$FORBIDDEN_DATA_DIR" ]] && verify_die "refusing shared data dir $FORBIDDEN_DATA_DIR"
    [[ -n "$config" && "$config" == "$FORBIDDEN_CONFIG" ]] && verify_die "refusing user config $FORBIDDEN_CONFIG"
    case "$socket$data_dir$config" in
        *'/.config/ployz/'* | */run/ployz/* | */var/lib/ployz*)
            verify_die "path looks like a user/system Ployz install: socket=$socket data_dir=$data_dir config=$config"
            ;;
    esac
}

verify_free_tcp_port() {
    python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

verify_binaries() {
    PLOYZ_BIN=${PLOYZ_BIN:-$VERIFY_REPO_ROOT/target/debug/ployz}
    PLOYZD_BIN=${PLOYZD_BIN:-$VERIFY_REPO_ROOT/target/debug/ployzd}
    if [[ ! -x "$PLOYZ_BIN" || ! -x "$PLOYZD_BIN" ]]; then
        (cd "$VERIFY_REPO_ROOT" && cargo build -p ployz -p ployzd --locked)
    fi
    [[ -x "$PLOYZ_BIN" ]] || verify_die "missing $PLOYZ_BIN"
    [[ -x "$PLOYZD_BIN" ]] || verify_die "missing $PLOYZD_BIN"
}
