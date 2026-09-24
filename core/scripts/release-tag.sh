#!/usr/bin/env bash

set -euo pipefail

beta_release_tag() {
    [[ "$1" =~ ^v[0-9]+\.[0-9]+\.[0-9]+-beta\.[0-9]+$ ]]
}

stable_release_tag() {
    [[ "$1" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]
}

channel_name_for_tag() {
    local tag=$1
    if beta_release_tag "$tag"; then
        printf 'beta\n'
    elif stable_release_tag "$tag"; then
        printf 'stable\n'
    else
        echo "tag '$tag' is not vX.Y.Z or vX.Y.Z-beta.N" >&2
        return 1
    fi
}

# Prints the tag's release line: v1.2.3 -> v1.
release_line_for_tag() {
    local tag=$1
    printf '%s\n' "${tag%%.*}"
}

# Succeeds when release tag $1 is higher than $2 by semver: vX.Y.Z outranks vX.Y.Z-beta.N.
release_tag_higher() {
    local -a left right
    read -ra left <<< "$(release_tag_rank "$1")"
    read -ra right <<< "$(release_tag_rank "$2")"
    local index
    for index in 0 1 2 3 4; do
        if ((left[index] != right[index])); then
            ((left[index] > right[index]))
            return
        fi
    done
    return 1
}

release_tag_rank() {
    [[ "$1" =~ ^v([0-9]+)\.([0-9]+)\.([0-9]+)(-beta\.([0-9]+))?$ ]] || {
        echo "tag '$1' is not vX.Y.Z or vX.Y.Z-beta.N" >&2
        return 1
    }
    if [ -n "${BASH_REMATCH[4]}" ]; then
        printf '%s %s %s 0 %s\n' "${BASH_REMATCH[@]:1:3}" "${BASH_REMATCH[5]}"
    else
        printf '%s %s %s 1 0\n' "${BASH_REMATCH[@]:1:3}"
    fi
}
