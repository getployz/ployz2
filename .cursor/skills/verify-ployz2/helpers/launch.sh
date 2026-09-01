#!/usr/bin/env bash
# Start one isolated, uninitialized ployzd plus an empty Ployz config.
# Usage: helpers/launch.sh <instance>
# Completion: run.env exists, unix socket accepts, GET /metrics returns this build.

set -euo pipefail
# shellcheck disable=SC1091
source "$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)/common.sh"

INSTANCE=${1:-}
verify_require_instance "$INSTANCE"
shift || true
[[ "$#" -eq 0 ]] || verify_die "launch.sh takes only the instance name"

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
METRICS_PORT=$(verify_free_tcp_port)
METRICS_ADDRESS=127.0.0.1:$METRICS_PORT

verify_refuse_shared "$SOCKET" "$DATA_DIR" "$CONFIG"

mkdir -p "$DATA_DIR" "$SOCKET_DIR" "$EVIDENCE_DIR"
: >"$CONFIG"
chmod 600 "$CONFIG"

# Isolate from any ambient Fast CI / user Ployz env.
unset PLOYZ_CONFIG PLOYZ_CONNECT PLOYZ_CONTEXT PLOYZ_AUTO_CONFIRM PLOYZ_DAEMON_VERSION || true

nohup "$PLOYZD_BIN" \
    --data-dir "$DATA_DIR" \
    --socket "$SOCKET" \
    --metrics-address "$METRICS_ADDRESS" \
    --log-level info \
    >"$LOG" 2>&1 &
PLOYZD_PID=$!

cleanup_failed_launch() {
    if kill -0 "$PLOYZD_PID" 2>/dev/null; then
        kill -TERM "$PLOYZD_PID" 2>/dev/null || true
        wait "$PLOYZD_PID" 2>/dev/null || true
    fi
}
trap cleanup_failed_launch ERR

deadline=$((SECONDS + 15))
while [[ ! -S "$SOCKET" ]]; do
    if ! kill -0 "$PLOYZD_PID" 2>/dev/null; then
        verify_die "ployzd exited before the socket appeared; log: $LOG"
    fi
    if [[ "$SECONDS" -ge "$deadline" ]]; then
        verify_die "socket did not appear within 15s: $SOCKET"
    fi
    sleep 0.05
done

python3 - "$SOCKET" <<'PY'
import socket, sys, time
path = sys.argv[1]
end = time.time() + 10
last = None
while time.time() < end:
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(0.2)
        s.connect(path)
        s.close()
        sys.exit(0)
    except OSError as error:
        last = error
        time.sleep(0.05)
print(f"unix socket did not accept: {last}", file=sys.stderr)
sys.exit(1)
PY

metrics_ok=0
while [[ "$SECONDS" -lt "$deadline" ]]; do
    if curl -fsS --max-time 1 "http://$METRICS_ADDRESS/metrics" | grep -Fq 'ployz_ployzd_build_info{version="'; then
        metrics_ok=1
        break
    fi
    sleep 0.05
done
[[ "$metrics_ok" -eq 1 ]] || verify_die "metrics did not become ready at $METRICS_ADDRESS"

CLI_VERSION=$("$PLOYZ_BIN" --version)
DAEMON_VERSION=$("$PLOYZD_BIN" version)
[[ "$CLI_VERSION" == "$DAEMON_VERSION" ]] || verify_die "CLI $CLI_VERSION != daemon $DAEMON_VERSION"

cat >"$ENV_FILE" <<EOF
INSTANCE=$INSTANCE
PLOYZD_PID=$PLOYZD_PID
PLOYZ_BIN=$PLOYZ_BIN
PLOYZD_BIN=$PLOYZD_BIN
DATA_DIR=$DATA_DIR
SOCKET=$SOCKET
CONFIG=$CONFIG
LOG=$LOG
METRICS_ADDRESS=$METRICS_ADDRESS
RUN_DIR=$RUN_DIR
EVIDENCE_DIR=$EVIDENCE_DIR
CLI_VERSION=$CLI_VERSION
REPO_ROOT=$VERIFY_REPO_ROOT
EOF

trap - ERR
printf 'launched instance %s pid %s socket %s metrics %s evidence %s\n' \
    "$INSTANCE" "$PLOYZD_PID" "$SOCKET" "$METRICS_ADDRESS" "$EVIDENCE_DIR"
