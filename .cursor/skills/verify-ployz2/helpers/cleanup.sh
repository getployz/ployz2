#!/usr/bin/env bash
# Tear down the instance this run created. Does not delete evidence.
# Usage: helpers/cleanup.sh <instance>
# Completion: pid is gone (or was already gone); run dir removed; evidence dir still present.

set -euo pipefail
# shellcheck disable=SC1091
source "$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)/common.sh"

INSTANCE=${1:-}
verify_require_instance "$INSTANCE"

ENV_FILE=$(verify_run_env "$INSTANCE")
EVIDENCE_DIR=$(verify_evidence_dir "$INSTANCE")
RUN_DIR=$(verify_instance_dir "$INSTANCE")

if [[ ! -f "$ENV_FILE" ]]; then
    echo "cleanup: no run.env for $INSTANCE; nothing to stop"
    [[ -d "$EVIDENCE_DIR" ]] && echo "evidence remains at $EVIDENCE_DIR"
    exit 0
fi

# shellcheck disable=SC1090
source "$ENV_FILE"
verify_refuse_shared "${SOCKET:-}" "${DATA_DIR:-}" "${CONFIG:-}"

if [[ -n "${PLOYZD_PID:-}" ]] && kill -0 "$PLOYZD_PID" 2>/dev/null; then
    kill -TERM "$PLOYZD_PID"
    deadline=$((SECONDS + 10))
    while kill -0 "$PLOYZD_PID" 2>/dev/null; do
        if [[ "$SECONDS" -ge "$deadline" ]]; then
            kill -KILL "$PLOYZD_PID" 2>/dev/null || true
            break
        fi
        sleep 0.1
    done
    wait "$PLOYZD_PID" 2>/dev/null || true
fi

rm -rf "$RUN_DIR"

if [[ -d "$EVIDENCE_DIR" ]]; then
    echo "cleaned instance $INSTANCE; evidence remains at $EVIDENCE_DIR"
else
    echo "cleaned instance $INSTANCE; evidence dir missing: $EVIDENCE_DIR" >&2
    exit 1
fi
