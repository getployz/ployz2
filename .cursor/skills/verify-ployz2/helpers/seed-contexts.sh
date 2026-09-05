#!/usr/bin/env bash
# Write an isolated two-context config in the product yaml shape (what machine init persists).
# Usage: helpers/seed-contexts.sh <instance>
# Completion: ctx ls against this config prints NAME/CURRENT/CONNECTIONS with prod current.

set -euo pipefail
# shellcheck disable=SC1091
source "$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)/common.sh"

INSTANCE=${1:-}
verify_require_instance "$INSTANCE"
verify_load_run "$INSTANCE"
verify_refuse_shared "$SOCKET" "$DATA_DIR" "$CONFIG"

cat >"$CONFIG" <<EOF
current_context: prod
contexts:
  dev:
    connections:
    - unix: $SOCKET
  prod:
    connections:
    - unix: $SOCKET
    - unix: $RUN_DIR/other.sock
EOF
chmod 600 "$CONFIG"

mkdir -p "$EVIDENCE_DIR"
cp "$CONFIG" "$EVIDENCE_DIR/seeded-config.yaml"
echo "seeded $CONFIG"
