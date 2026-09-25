#!/usr/bin/env bash
# Readies the runner (Docker, ployz), then checks in with Ployz Cloud. Runs before checkout.
set -euo pipefail

# shellcheck source=oidc.sh
source "$(dirname "$0")/oidc.sh"
work="$RUNNER_TEMP/ployz-build"
fail() {
    echo "::error::$1"
    exit 1
}

[[ "$(uname -s)" == Linux ]] || fail "Ployz builds need a Linux runner."
[[ "$PLOYZ_BUILD_ID" =~ ^[A-Za-z0-9_-]{1,128}$ ]] || fail "Invalid build id."
cloud=${PLOYZ_CLOUD%/}
[[ "$cloud" =~ ^https?://[A-Za-z0-9.-]+(:[0-9]{1,5})?$ ]] || fail "Invalid Cloud URL; expected an origin like https://ployz.dev."
[[ "$PLOYZ_VERSION" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-beta\.(0|[1-9][0-9]*))?$ ]] || fail "Invalid ployz version."
[[ -n "${ACTIONS_ID_TOKEN_REQUEST_URL:-}" ]] || fail "The workflow needs 'permissions: id-token: write'."

umask 077
mkdir -p "$work/bin"

# ployz-build refuses Docker without the containerd image store; GitHub's hosted runners ship without it.
has_containerd_store() {
    docker info --format '{{json .DriverStatus}}' | grep -q 'io.containerd.snapshotter.v1'
}
if ! has_containerd_store; then
    echo "Enabling Docker's containerd image store"
    config=/etc/docker/daemon.json
    if sudo test -s "$config"; then sudo cat "$config"; else echo '{}'; fi |
        jq '.features["containerd-snapshotter"] = true' >"$work/daemon.json"
    sudo install -m 0644 "$work/daemon.json" "$config"
    sudo systemctl restart docker
    has_containerd_store || fail "Docker's containerd image store could not be enabled."
fi

# The exact version Cloud runs, so the build fingerprint matches.
curl -fsSL https://ployz.sh -o "$work/install.sh"
PLOYZ_VERSION="$PLOYZ_VERSION" INSTALL_BIN_DIR="$work/bin" sh "$work/install.sh"
echo "$work/bin" >>"$GITHUB_PATH"

# GitHub's OIDC token proves this run to Cloud; its audience is the Cloud origin.
oidc_token "$cloud"

curl -fsS --retry 3 -X POST -H "Authorization: Bearer $token" \
    -o "$work/check-in.json" "$cloud/api/builds/$PLOYZ_BUILD_ID/check-in" ||
    fail "Ployz Cloud refused the check-in for build $PLOYZ_BUILD_ID."

# Mask the grant and every build secret before anything can print them. Masks are per line.
jq -r '.grant, (.deployment.snapshots[]?.resolvedEnv // {} | .[])' "$work/check-in.json" |
    while IFS= read -r line; do
        if [[ -n "$line" ]]; then echo "::add-mask::$line"; fi
    done

jq -e '.grant | type == "string" and startswith("ployzgrant1:")' "$work/check-in.json" >/dev/null ||
    fail "Check-in response has no Build Grant."
commit=$(jq -r '.commit' "$work/check-in.json")
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || fail "Check-in response has no commit."
[[ "$(jq -r '.fingerprint' "$work/check-in.json")" =~ ^[0-9a-f]{64}$ ]] ||
    fail "Check-in response has no fingerprint."
jq -e '.deployment | objects' "$work/check-in.json" >"$work/deployment.json" ||
    fail "Check-in response has no deployment."

echo "commit=$commit" >>"$GITHUB_OUTPUT"
