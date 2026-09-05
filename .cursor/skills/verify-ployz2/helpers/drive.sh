#!/usr/bin/env bash
# Run one ployz invocation against this instance's isolated config.
# Usage: helpers/drive.sh <instance> [--connect-unix] <ployz-args...>
# Completion: transcript files exist under the instance evidence dir; exit status is ployz's.

set -euo pipefail
# shellcheck disable=SC1091
source "$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)/common.sh"

INSTANCE=${1:-}
verify_require_instance "$INSTANCE"
shift
verify_load_run "$INSTANCE"
verify_refuse_shared "$SOCKET" "$DATA_DIR" "$CONFIG"
# CLI-only instances use helpers/prepare.sh (no PLOYZD_PID). Daemon-backed drives use launch.sh.

CONNECT_UNIX=0
if [[ "${1:-}" == --connect-unix ]]; then
    CONNECT_UNIX=1
    shift
fi
[[ "$#" -gt 0 ]] || verify_die "drive.sh needs ployz arguments after the instance name"

mkdir -p "$EVIDENCE_DIR"
stamp=$(date -u +%Y%m%dT%H%M%SZ)
seq=$(printf '%03d' "$(find "$EVIDENCE_DIR" -maxdepth 1 -name 'drive-*-cmd.txt' | wc -l)")
prefix=$EVIDENCE_DIR/drive-${seq}-${stamp}

args=("$PLOYZ_BIN" --ployz-config "$CONFIG")
if [[ "$CONNECT_UNIX" -eq 1 ]]; then
    args+=(--connect "unix://$SOCKET")
fi
args+=("$@")

printf '%s\n' "${args[*]}" >"${prefix}-cmd.txt"
cp "$CONFIG" "${prefix}-config-before.yaml"

unset PLOYZ_CONFIG PLOYZ_CONNECT PLOYZ_CONTEXT PLOYZ_DAEMON_VERSION || true
# Leave PLOYZ_AUTO_CONFIRM untouched so callers can opt in.

set +e
"${args[@]}" >"${prefix}-stdout.txt" 2>"${prefix}-stderr.txt"
status=$?
set -e
printf '%s\n' "$status" >"${prefix}-exit.txt"
cp "$CONFIG" "${prefix}-config-after.yaml"

{
    echo "cmd: ${args[*]}"
    echo "exit: $status"
    echo "stdout: ${prefix}-stdout.txt"
    echo "stderr: ${prefix}-stderr.txt"
    echo "config_before: ${prefix}-config-before.yaml"
    echo "config_after: ${prefix}-config-after.yaml"
} >"${prefix}-summary.txt"

echo "$prefix status=$status"
exit "$status"
