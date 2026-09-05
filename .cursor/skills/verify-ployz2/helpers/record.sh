#!/usr/bin/env bash
# Capture any command into this instance's evidence dir (installer scripts, cargo tests).
# Usage: helpers/record.sh <instance> [--cwd DIR] -- <command...>
# Completion: transcript files exist; exit status is the command's.

set -euo pipefail
# shellcheck disable=SC1091
source "$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)/common.sh"

INSTANCE=${1:-}
verify_require_instance "$INSTANCE"
shift

CWD=
if [[ "${1:-}" == --cwd ]]; then
    CWD=${2:-}
    [[ -n "$CWD" ]] || verify_die "record.sh --cwd needs a directory"
    shift 2
fi
if [[ "${1:-}" == -- ]]; then
    shift
fi
[[ "$#" -gt 0 ]] || verify_die "record.sh needs a command after --"

ENV_FILE=$(verify_run_env "$INSTANCE")
if [[ -f "$ENV_FILE" ]]; then
    verify_load_run "$INSTANCE"
    verify_refuse_shared "$SOCKET" "$DATA_DIR" "$CONFIG"
else
    EVIDENCE_DIR=$(verify_evidence_dir "$INSTANCE")
    CONFIG=
    SOCKET=
    DATA_DIR=
fi

mkdir -p "$EVIDENCE_DIR"
stamp=$(date -u +%Y%m%dT%H%M%SZ)
seq=$(printf '%03d' "$(find "$EVIDENCE_DIR" -maxdepth 1 -name 'record-*-cmd.txt' | wc -l)")
prefix=$EVIDENCE_DIR/record-${seq}-${stamp}

{
    printf 'cwd: %s\n' "${CWD:-$(pwd)}"
    printf 'cmd:'
    printf ' %q' "$@"
    printf '\n'
} >"${prefix}-cmd.txt"

if [[ -n "${CONFIG:-}" && -f "$CONFIG" ]]; then
    cp "$CONFIG" "${prefix}-config-before.yaml"
fi

unset PLOYZ_CONFIG PLOYZ_CONNECT PLOYZ_CONTEXT PLOYZ_DAEMON_VERSION || true

set +e
if [[ -n "$CWD" ]]; then
    (cd "$CWD" && "$@") >"${prefix}-stdout.txt" 2>"${prefix}-stderr.txt"
    status=$?
else
    "$@" >"${prefix}-stdout.txt" 2>"${prefix}-stderr.txt"
    status=$?
fi
set -e
printf '%s\n' "$status" >"${prefix}-exit.txt"
if [[ -n "${CONFIG:-}" && -f "$CONFIG" ]]; then
    cp "$CONFIG" "${prefix}-config-after.yaml"
fi

{
    echo "cmd: $*"
    echo "cwd: ${CWD:-$(pwd)}"
    echo "exit: $status"
    echo "stdout: ${prefix}-stdout.txt"
    echo "stderr: ${prefix}-stderr.txt"
} >"${prefix}-summary.txt"

echo "$prefix status=$status"
exit "$status"
