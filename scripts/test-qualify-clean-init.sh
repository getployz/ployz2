#!/usr/bin/env bash

set -euo pipefail

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
LOG=$TMP/calls.log
export LOG

cat > "$TMP/ployz" <<'EOF'
#!/bin/sh
printf 'ployz' >> "$LOG"
printf ' <%s>' "$@" >> "$LOG"
printf '\n' >> "$LOG"
[ "$3" != root@two ]
EOF
cat > "$TMP/ssh" <<'EOF'
#!/bin/sh
printf 'ssh <%s>\n' "$1" >> "$LOG"
EOF
chmod 0755 "$TMP/ployz" "$TMP/ssh"

if PLOYZ_BIN="$TMP/ployz" SSH_BIN="$TMP/ssh" PLOYZ_QUALIFY_RUN=run \
    bash "$ROOT/scripts/qualify-clean-init.sh" 0.1.2-beta.23 \
        root@one root@two root@three >"$TMP/output" 2>&1; then
    echo "qualification passed despite a failed init" >&2
    exit 1
fi

grep -Fxq 'ployz <machine> <init> <root@one> <--context> <qualify-run-1> <--name> <qualify-1> <--version> <0.1.2-beta.23> <--storage> <none> <--no-dns> <--yes>' "$LOG"
grep -Fxq 'ployz <machine> <init> <root@two> <--context> <qualify-run-2> <--name> <qualify-2> <--version> <0.1.2-beta.23> <--storage> <none> <--no-dns> <--yes>' "$LOG"
grep -Fxq 'ployz <machine> <init> <root@three> <--context> <qualify-run-3> <--name> <qualify-3> <--version> <0.1.2-beta.23> <--storage> <none> <--no-dns> <--yes>' "$LOG"
grep -Fxq 'ssh <root@two>' "$LOG"
grep -Fq '1/3 clean founder initializations failed' "$TMP/output"

echo "clean-init qualification interface passed"
