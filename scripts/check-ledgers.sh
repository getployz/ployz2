#!/bin/sh
set -eu

unset CDPATH
repo_root=$(cd "$(dirname "$0")/.." && pwd)
todo_ledger="$repo_root/UPSTREAM_TODOS.md"
baseline=b7e224a1eff98813b1d1a32034d977be24be994e

fail() {
  echo "ledger check failed: $*" >&2
  exit 1
}

explicit_count=$(grep -Ec '^\| UT-[0-9]{3} \|' "$todo_ledger")
[ "$explicit_count" -eq 151 ] || fail "expected 151 explicit TODO rows, found $explicit_count"

omission_count=$(grep -Ec '^\| EO-[0-9]{3} \|' "$todo_ledger")
[ "$omission_count" -eq 19 ] || fail "expected 19 equivalent omission rows, found $omission_count"

ledger_rows=$(grep -E '^\| (UT|EO)-[0-9]{3} \|' "$todo_ledger")
unpinned=$(printf '%s\n' "$ledger_rows" | grep -Fvc "$baseline" || true)
[ "$unpinned" -eq 0 ] || fail "$unpinned TODO ledger rows are not pinned to the baseline"

bad_dispositions=$(printf '%s\n' "$ledger_rows" | grep -Evc '\| (Preserve boundary|Carry TODO|Resolve by Rust structure|Migration cleanup / not applicable|Reference only) \|' || true)
[ "$bad_dispositions" -eq 0 ] || fail "$bad_dispositions TODO ledger rows have an invalid disposition"

bad_ledger_keys=$(printf '%s\n' "$ledger_rows" | awk -F '|' '
  {
    key = $2
    gsub(/^ +| +$/, "", key)
    prefix = substr(key, 1, 2)
    count[prefix]++
    expected = sprintf("%s-%03d", prefix, count[prefix])
    if (key != expected) bad++
  }
  END {print bad + 0}
')
[ "$bad_ledger_keys" -eq 0 ] || fail "$bad_ledger_keys TODO ledger keys are duplicate or out of sequence"

pending_locations=$(printf '%s\n' "$ledger_rows" | grep -Fc 'Pending — owning implementation slice' || true)
[ "$pending_locations" -eq 0 ] || fail "$pending_locations TODO ledger rows still have a pending location"

bad_deviation_lines=$(grep -Evc '^- .+' "$repo_root/CLI_DEVIATIONS.md" || true)
[ "$bad_deviation_lines" -eq 0 ] || fail "$bad_deviation_lines malformed CLI deviation lines"

echo "ledgers verified: 151 TODO markers, 19 equivalent omissions, CLI deviation format"
