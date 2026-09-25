#!/usr/bin/env bash
# Builds the checked-out commit and pushes it into the Machine named by the Build Grant,
# reporting the Build Steps to Ployz Cloud as they happen and once more when the build ends,
# whether or not it succeeded.
set -euo pipefail

# shellcheck source=oidc.sh
source "$(dirname "$0")/oidc.sh"
work="$RUNNER_TEMP/ployz-build"
cloud=${PLOYZ_CLOUD%/}
events="$work/events.jsonl"
# How often, in seconds, new Build Steps go to Cloud while the build runs.
interval=${PLOYZ_STEPS_INTERVAL:-5}
PLOYZ_BUILD_GRANT=$(jq -r '.grant' "$work/check-in.json")
export PLOYZ_BUILD_GRANT
: >"$events"
sent=0

# Posts the complete event lines not sent yet, from line $sent; with $1, the platforms built, the
# build's end. A fresh OIDC token proves this run each time. Best effort: a refused batch is resent.
report() {
    local lines
    lines=$(wc -l <"$events")
    [[ -n "${1:-}" || $lines -gt $sent ]] || return 0
    tail -n "+$((sent + 1))" "$events" | head -n "$((lines - sent))" |
        jq -cs --argjson from "$sent" --argjson platforms "${1:-null}" \
            '{from: $from, events: .} + (if $platforms == null then {} else {platforms: $platforms} end)' >"$work/steps.json"
    oidc_token "$cloud"
    if curl -fsS --retry 3 -X POST -H "Authorization: Bearer $token" -H "Content-Type: application/json" \
        --data-binary "@$work/steps.json" "$cloud/api/builds/$PLOYZ_BUILD_ID/steps" >/dev/null; then
        sent=$lines
    else
        echo "::warning::Ployz Cloud did not accept the Build Steps."
    fi
}

# stdout is one JSON line; build logs go to stderr and, as events, to a file.
ployz build \
    --deployment "$work/deployment.json" \
    --commit "$(jq -r '.commit' "$work/check-in.json")" \
    --fingerprint "$(jq -r '.fingerprint' "$work/check-in.json")" \
    --source "$GITHUB_WORKSPACE" \
    --events "$events" >"$work/result.json" &
build=$!
while kill -0 "$build" 2>/dev/null; do
    sleep "$interval"
    report
done
status=0
wait "$build" || status=$?

# Cloud takes the digest from the Machine, not from here; the platforms end the report.
report "$(if [[ $status == 0 ]]; then jq -c '.platforms' "$work/result.json"; else echo '[]'; fi)"
[[ $status == 0 ]] || exit "$status"
digest=$(jq -r '.digest' "$work/result.json")
echo "digest=$digest" >>"$GITHUB_OUTPUT"
echo "Pushed \`$digest\` to the Machine." >>"$GITHUB_STEP_SUMMARY"
