#!/usr/bin/env bash

set -euo pipefail

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
# shellcheck source=scripts/homebrew-formula.sh
source "$ROOT/scripts/homebrew-formula.sh"
# shellcheck source=scripts/repoint-homebrew-tap.sh
PLOYZ_HOMEBREW_TEST_ONLY=true source "$ROOT/scripts/repoint-homebrew-tap.sh"

HOMEBREW_TAP_REPO=${HOMEBREW_TAP_REPO:-getployz/homebrew-ployz}

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

write_channel_file() {
    local dest_dir=$1 tag=$2 channel
    channel=$(channel_name_for_tag "$tag")
    mkdir -p "$dest_dir"
    printf '%s\n' "$tag" > "$dest_dir/$channel"
}

git_identity() {
    git -C "$1" config user.name "github-actions[bot]"
    git -C "$1" config user.email "41898282+github-actions[bot]@users.noreply.github.com"
}

commit_if_changed() {
    local work=$1 message=$2
    git -C "$work" add -A
    if git -C "$work" diff --cached --quiet; then
        return 0
    fi
    git -C "$work" commit -m "$message"
}

clone_or_init_branch() {
    local remote=$1 branch=$2 work=$3
    if git ls-remote --heads "$remote" "$branch" | grep -q .; then
        git clone --depth 1 --branch "$branch" "$remote" "$work"
        return
    fi
    mkdir -p "$work"
    git -C "$work" init
    git -C "$work" checkout -b "$branch"
    git -C "$work" remote add origin "$remote"
}

push_channel_file() {
    local tag=$1
    local token=${GITHUB_TOKEN:-${GH_TOKEN:-}}
    local repo=${GITHUB_REPOSITORY:-getployz/ployz2}
    local remote work
    [ -n "$token" ] || {
        echo "GITHUB_TOKEN is required to update the channels branch" >&2
        return 1
    }
    remote="https://x-access-token:${token}@github.com/${repo}.git"
    work=$(mktemp -d)
    clone_or_init_branch "$remote" channels "$work"
    git_identity "$work"
    write_channel_file "$work" "$tag"
    commit_if_changed "$work" "channel $(channel_name_for_tag "$tag") -> $tag"
    git -C "$work" push origin "HEAD:channels"
    rm -rf "$work"
}

push_homebrew_tap() {
    local tag=$1 checksums=$2
    local token=${HOMEBREW_TAP_TOKEN:-}
    local version=${tag#v} work formula
    [ -n "$token" ] || {
        echo "HOMEBREW_TAP_TOKEN is required to update $HOMEBREW_TAP_REPO" >&2
        return 1
    }
    work=$(mktemp -d)
    git clone --depth 1 "https://x-access-token:${token}@github.com/${HOMEBREW_TAP_REPO}.git" "$work"
    git_identity "$work"
    formula=$(mktemp)
    write_homebrew_formula_from_checksums "$checksums" "$formula" "$version" "$tag" "${GITHUB_REPOSITORY:-getployz/ployz2}"
    repoint_homebrew_tap "$work" "$formula"
    commit_if_changed "$work" "ployz $version"
    git -C "$work" push origin HEAD
    rm -f "$formula"
    rm -rf "$work"
}

promote_published_release() {
    local tag=$1 checksums_dir checksums formula
    if [ -n "${PLOYZ_CHANNELS_DIR:-}" ]; then
        write_channel_file "$PLOYZ_CHANNELS_DIR" "$tag"
    else
        push_channel_file "$tag"
    fi
    if ! stable_release_tag "$tag"; then
        return 0
    fi
    if [ -n "${PLOYZ_HOMEBREW_TAP_DIR:-}" ]; then
        checksums=${PLOYZ_CHECKSUMS:-}
        [ -n "$checksums" ] || {
            echo "PLOYZ_CHECKSUMS is required when PLOYZ_HOMEBREW_TAP_DIR is set" >&2
            return 1
        }
        formula=$(mktemp)
        write_homebrew_formula_from_checksums "$checksums" "$formula" "${tag#v}" "$tag" "${GITHUB_REPOSITORY:-getployz/ployz2}"
        repoint_homebrew_tap "$PLOYZ_HOMEBREW_TAP_DIR" "$formula"
        rm -f "$formula"
        return 0
    fi
    checksums_dir=$(mktemp -d)
    gh release download "$tag" --pattern checksums.txt --dir "$checksums_dir"
    push_homebrew_tap "$tag" "$checksums_dir/checksums.txt"
    rm -rf "$checksums_dir"
}

if [ "${PLOYZ_PROMOTE_TEST_ONLY:-false}" != true ]; then
    tag=${1:-}
    [ -n "$tag" ] || {
        echo "usage: $0 <tag>" >&2
        exit 1
    }
    promote_published_release "$tag"
fi
