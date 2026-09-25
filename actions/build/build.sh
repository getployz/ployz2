#!/usr/bin/env bash
# Builds the checked-out commit and pushes it into the Machine named by the Build Grant,
# then reports the Build Steps to Ployz Cloud, whether or not the build succeeded.
set -euo pipefail

work="$RUNNER_TEMP/ployz-build"
cloud=${PLOYZ_CLOUD%/}
PLOYZ_BUILD_GRANT=$(jq -r '.grant' "$work/check-in.json")
export PLOYZ_BUILD_GRANT

# stdout is one JSON line; build logs go to stderr and, as events, to a file.
status=0
result=$(ployz build \
    --deployment "$work/deployment.json" \
    --commit "$(jq -r '.commit' "$work/check-in.json")" \
    --fingerprint "$(jq -r '.fingerprint' "$work/check-in.json")" \
    --source "$GITHUB_WORKSPACE" \
    --events "$work/events.jsonl") || status=$?

# A fresh OIDC token proves this run again; Cloud takes the digest from the Machine, not from here.
audience=$(jq -rn --arg value "$cloud" '$value | @uri')
token=$(curl -fsS -H "Authorization: bearer $ACTIONS_ID_TOKEN_REQUEST_TOKEN" \
    "$ACTIONS_ID_TOKEN_REQUEST_URL&audience=$audience" | jq -r '.value')
echo "::add-mask::$token"
touch "$work/events.jsonl"
platforms=$(if [[ $status == 0 ]]; then jq -c '.platforms' <<<"$result"; else echo '[]'; fi)
jq -cs --argjson platforms "$platforms" '{events: ., platforms: $platforms}' "$work/events.jsonl" >"$work/steps.json"
curl -fsS --retry 3 -X POST -H "Authorization: Bearer $token" -H "Content-Type: application/json" \
    --data-binary "@$work/steps.json" "$cloud/api/builds/$PLOYZ_BUILD_ID/steps" >/dev/null ||
    echo "::warning::Ployz Cloud did not accept the Build Steps."

[[ $status == 0 ]] || exit "$status"
digest=$(jq -r '.digest' <<<"$result")
echo "digest=$digest" >>"$GITHUB_OUTPUT"
echo "Pushed \`$digest\` to the Machine." >>"$GITHUB_STEP_SUMMARY"
