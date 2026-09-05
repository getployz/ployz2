#!/usr/bin/env bash
# Read-only check that this instance is ours, live, and not Nick's cluster.
# Usage: helpers/doctor.sh <instance>
# Completion: prints OK and writes evidence/doctor.txt; exits 1 if any check fails.

set -euo pipefail
# shellcheck disable=SC1091
source "$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)/common.sh"

INSTANCE=${1:-}
verify_require_instance "$INSTANCE"
verify_load_run "$INSTANCE"
verify_refuse_shared "$SOCKET" "$DATA_DIR" "$CONFIG"
[[ -n "${PLOYZD_PID:-}" ]] || verify_die "doctor needs a launched daemon; run helpers/launch.sh $INSTANCE"

report=$(mktemp)
trap 'rm -f "$report"' EXIT

fail=0
check() {
    local name=$1
    shift
    if "$@"; then
        printf 'PASS %s\n' "$name" | tee -a "$report"
    else
        printf 'FAIL %s\n' "$name" | tee -a "$report"
        fail=1
    fi
}

pid_alive() { kill -0 "$PLOYZD_PID" 2>/dev/null; }
cmdline_has_socket() { tr '\0' ' ' <"/proc/$PLOYZD_PID/cmdline" | grep -Fq -- "--socket $SOCKET"; }
cmdline_has_data() { tr '\0' ' ' <"/proc/$PLOYZD_PID/cmdline" | grep -Fq -- "--data-dir $DATA_DIR"; }
socket_is_ours() { [[ -S "$SOCKET" ]]; }
not_system_socket() { [[ "$SOCKET" != "$FORBIDDEN_SOCKET" ]]; }
not_system_data() { [[ "$DATA_DIR" != "$FORBIDDEN_DATA_DIR" ]]; }
not_user_config() { [[ "$CONFIG" != "$FORBIDDEN_CONFIG" ]]; }
config_under_run() { [[ "$CONFIG" == "$RUN_DIR"/* ]]; }
versions_match() {
    local cli daemon metrics
    cli=$("$PLOYZ_BIN" --version)
    daemon=$("$PLOYZD_BIN" version)
    metrics=$(curl -fsS --max-time 2 "http://$METRICS_ADDRESS/metrics")
    [[ "$cli" == "$CLI_VERSION" && "$daemon" == "$CLI_VERSION" ]] &&
        grep -Fq "ployz_ployzd_build_info{version=\"$CLI_VERSION\"} 1" <<<"$metrics"
}
log_shows_uninitialized() { grep -Eq 'started' "$LOG" && grep -Eq 'uninitialized' "$LOG"; }
metrics_port_is_loopback() { [[ "$METRICS_ADDRESS" == 127.0.0.1:* ]]; }

check pid_alive pid_alive
check cmdline_socket cmdline_has_socket
check cmdline_data_dir cmdline_has_data
check socket_exists socket_is_ours
check not_system_socket not_system_socket
check not_system_data not_system_data
check not_user_config not_user_config
check config_isolated config_under_run
check versions_match versions_match
check log_uninitialized log_shows_uninitialized
check metrics_loopback metrics_port_is_loopback

mkdir -p "$EVIDENCE_DIR"
cp "$LOG" "$EVIDENCE_DIR/ployzd.log"
curl -fsS --max-time 2 "http://$METRICS_ADDRESS/metrics" >"$EVIDENCE_DIR/metrics.txt"
{
    echo "instance=$INSTANCE pid=$PLOYZD_PID version=$CLI_VERSION"
    echo "socket=$SOCKET"
    echo "data_dir=$DATA_DIR"
    echo "config=$CONFIG"
    echo "metrics=$METRICS_ADDRESS"
    echo "log=$LOG"
    cat "$report"
} >"$EVIDENCE_DIR/doctor.txt"

if [[ "$fail" -ne 0 ]]; then
    echo "doctor failed; see $EVIDENCE_DIR/doctor.txt" >&2
    exit 1
fi
echo "OK instance=$INSTANCE pid=$PLOYZD_PID version=$CLI_VERSION"
