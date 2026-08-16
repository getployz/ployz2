#!/usr/bin/env bash

set -euo pipefail

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
action="$ROOT/.github/actions/build-testkit-image/action.yml"
workflow="$ROOT/.github/workflows/ci.yml"
dockerfile="$ROOT/ployz-testkit/Dockerfile"

assert_contains() {
    if ! grep -Fq -- "$2" "$1"; then
        echo "$1 does not contain: $2" >&2
        exit 1
    fi
}

assert_not_contains() {
    if grep -Fq -- "$2" "$1"; then
        echo "$1 still contains: $2" >&2
        exit 1
    fi
}

assert_not_contains "$dockerfile" "FROM rust"
assert_contains "$dockerfile" "COPY ployz-testkit/prebuilt/ployz /usr/local/bin/ployz"
assert_contains "$dockerfile" "COPY ployz-testkit/prebuilt/ployzd /usr/local/bin/ployzd"

assert_contains "$action" "Swatinem/rust-cache@v2"
assert_contains "$action" "shared-key: testkit-release"
assert_contains "$action" "cargo build --locked --release --package ployz --package ployzd"
assert_contains "$action" "ployz-testkit/prebuilt/ployz"
assert_contains "$action" "ployz-testkit/prebuilt/ployzd"
assert_contains "$action" "type=registry,ref=ghcr.io/getployz/ployz2-testkit:main"
assert_not_contains "$action" "mode=max"
assert_not_contains "$action" "type=gha,scope=ployz-testkit"

assert_contains "$workflow" "bash scripts/test-testkit-image.sh"
assert_contains "$ROOT/.gitignore" "ployz-testkit/prebuilt/"

for job in layer3-image-amd64 layer3-image-arm64; do
    if ! awk -v job="$job" '
        $0 == "  " job ":" { in_job = 1 }
        in_job && $0 == "      actions: write" { found = 1 }
        in_job && /^  [a-z]/ && $0 != "  " job ":" { in_job = 0 }
        END { exit found ? 0 : 1 }
    ' "$workflow"; then
        echo "$job must have actions: write for rust-cache" >&2
        exit 1
    fi
done
