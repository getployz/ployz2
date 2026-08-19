#!/usr/bin/env bash

set -euo pipefail

run_name_for_tag() {
    printf 'Release %s\n' "$1"
}

newest_run_id_named_except() {
    local name=$1 except=$2
    jq -r --arg name "$name" --arg except "$except" '
        ($except | split("\n") | map(select(length > 0) | tostring)) as $skip
        | [.[] | select(.displayTitle == $name) | select((.databaseId | tostring) as $id | ($skip | index($id) | not))]
        | sort_by(.createdAt)
        | reverse
        | .[0].databaseId // empty
    '
}

newest_run_id_named() {
    newest_run_id_named_except "$1" ""
}

if [ "${PLOYZ_BOUNCE_RELEASE_TEST_ONLY:-false}" != true ]; then
    tag=${1:-}
    sha=${2:-}
    ref=${3:-main}
    workflow=${4:-release.yml}
    [ -n "$tag" ] && [ -n "$sha" ] || {
        echo "usage: $0 <tag> <sha> [ref] [workflow]" >&2
        exit 1
    }
    existing=$(gh run list --workflow="$workflow" --event=workflow_dispatch --limit 20 --json databaseId --jq '.[].databaseId')
    gh workflow run "$workflow" --ref "$ref" -f "tag=$tag" -f "sha=$sha"
    wanted=$(run_name_for_tag "$tag")
    id=
    for _ in $(seq 1 30); do
        id=$(gh run list --workflow="$workflow" --event=workflow_dispatch --limit 20 --json databaseId,displayTitle,createdAt | newest_run_id_named_except "$wanted" "$existing")
        if [ -n "$id" ]; then
            gh run watch "$id" --exit-status
            exit $?
        fi
        sleep 2
    done
    echo "timed out waiting for $wanted on $ref" >&2
    exit 1
fi
