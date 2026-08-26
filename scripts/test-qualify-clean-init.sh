#!/usr/bin/env bash

set -euo pipefail

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
LOG=$TMP/calls.log
export LOG
diagnostics='systemctl status ployz.service --no-pager || true; journalctl -u ployz.service --no-pager -n 200 || true; docker inspect ployz-corrosion || true; docker logs --tail 200 ployz-corrosion || true; ss -H -lntup || true'

cat > "$TMP/ployz" <<'EOF'
#!/bin/sh
printf 'ployz' >> "$LOG"
printf ' <%s>' "$@" >> "$LOG"
printf '\n' >> "$LOG"
[ "$3" != root@two:2222 ]
EOF
cat > "$TMP/ssh" <<'EOF'
#!/bin/sh
printf 'ssh' >> "$LOG"
printf ' <%s>' "$@" >> "$LOG"
printf '\n' >> "$LOG"
EOF
chmod 0755 "$TMP/ployz" "$TMP/ssh"

set +e
PLOYZ_BIN="$TMP/ployz" SSH_BIN="$TMP/ssh" PLOYZ_QUALIFY_RUN=run \
    bash "$ROOT/scripts/qualify-clean-init.sh" 0.1.2-beta.23 \
        root@one 'root@two:2222' root@three >"$TMP/output" 2>&1
status=$?
set -e
if [ "$status" -ne 1 ]; then
    echo "qualification exited $status, expected 1" >&2
    exit 1
fi

grep -Fxq 'ployz <machine> <init> <root@one> <--context> <qualify-run-1> <--name> <qualify-1> <--version> <0.1.2-beta.23> <--storage> <none> <--no-dns>' "$LOG"
grep -Fxq 'ployz <machine> <init> <root@two:2222> <--context> <qualify-run-2> <--name> <qualify-2> <--version> <0.1.2-beta.23> <--storage> <none> <--no-dns>' "$LOG"
grep -Fxq 'ployz <machine> <init> <root@three> <--context> <qualify-run-3> <--name> <qualify-3> <--version> <0.1.2-beta.23> <--storage> <none> <--no-dns>' "$LOG"
if grep -q -- '--yes' "$LOG"; then
    echo "qualifier passed --yes, which confirms Machine reset" >&2
    exit 1
fi
grep -Fxq "ssh <-p> <2222> <root@two> <$diagnostics>" "$LOG"
grep -Fq '1/3 clean founder initializations failed' "$TMP/output"

echo "clean-init qualification interface passed"
