#!/usr/bin/env bash
# Builds the checked-out commit and pushes it into the Machine named by the Build Grant.
set -euo pipefail

work="$RUNNER_TEMP/ployz-build"
PLOYZ_BUILD_GRANT=$(jq -r '.grant' "$work/check-in.json")
export PLOYZ_BUILD_GRANT

# stdout is one JSON line; build logs go to stderr.
result=$(ployz build \
    --deployment "$work/deployment.json" \
    --commit "$(jq -r '.commit' "$work/check-in.json")" \
    --fingerprint "$(jq -r '.fingerprint' "$work/check-in.json")" \
    --source "$GITHUB_WORKSPACE")
digest=$(jq -r '.digest' <<<"$result")

echo "digest=$digest" >>"$GITHUB_OUTPUT"
echo "Pushed \`$digest\` to the Machine." >>"$GITHUB_STEP_SUMMARY"
