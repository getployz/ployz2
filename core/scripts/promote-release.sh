#!/usr/bin/env bash

set -euo pipefail

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
# shellcheck source=scripts/homebrew-formula.sh
source "$ROOT/scripts/homebrew-formula.sh"
# shellcheck source=scripts/release-tag.sh
source "$ROOT/scripts/release-tag.sh"

# Moves pointer file $1 to tag $2 only when the tag is higher, so an older-line fix never moves
# a channel backwards. A corrupt pointer stops the release rather than being overwritten.
advance_pointer() {
    local file=$1 tag=$2 current=
    if [ -f "$file" ]; then
        current=$(tr -d '[:space:]' < "$file")
        channel_name_for_tag "$current" > /dev/null || exit 1
        release_tag_higher "$tag" "$current" || return 0
    fi
    mkdir -p "$(dirname "$file")"
    printf '%s\n' "$tag" > "$file"
}

# Daemons read their line's pointer (v0/stable); only install.sh reads the unscoped one.
# beta is the highest release, so a stable tag advances it too.
write_channel_files() {
    local dest_dir=$1 tag=$2 line pointer pointers=beta
    line=${tag%%.*}
    [ "$(channel_name_for_tag "$tag")" = beta ] || pointers="stable beta"
    for pointer in $pointers; do
        advance_pointer "$dest_dir/$line/$pointer" "$tag"
        advance_pointer "$dest_dir/$pointer" "$tag"
    done
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

push_channel_files() {
    local work=$1 tag=$2
    local token=${GITHUB_TOKEN:-${GH_TOKEN:-}}
    local repo=${GITHUB_REPOSITORY:-getployz/ployz2}
    local remote
    [ -n "$token" ] || {
        echo "GITHUB_TOKEN is required to update the channels branch" >&2
        return 1
    }
    remote="https://x-access-token:${token}@github.com/${repo}.git"
    if git ls-remote --heads "$remote" channels | grep -q .; then
        git clone --depth 1 --branch channels "$remote" "$work"
    else
        git -C "$work" init
        git -C "$work" checkout -b channels
        git -C "$work" remote add origin "$remote"
    fi
    git_identity "$work"
    write_channel_files "$work" "$tag"
    commit_if_changed "$work" "channels -> $tag"
    git -C "$work" push origin "HEAD:channels"
}

dispatch_ployz_sh_site() {
    # channels has no workflow file, so a push there cannot deploy Pages.
    gh workflow run ployz-sh.yml --ref main
}

push_homebrew_tap() {
    local tag=$1 checksums=$2
    local token=${HOMEBREW_TAP_TOKEN:-}
    local version=${tag#v} work
    [ -n "$token" ] || {
        echo "HOMEBREW_TAP_TOKEN is required to update getployz/homebrew-ployz" >&2
        return 1
    }
    work=$(mktemp -d)
    git clone --depth 1 "https://x-access-token:${token}@github.com/getployz/homebrew-ployz.git" "$work"
    git_identity "$work"
    write_homebrew_formula_from_checksums "$checksums" "$work/Formula/ployz.rb" "$version" "$tag" "${GITHUB_REPOSITORY:-getployz/ployz2}"
    commit_if_changed "$work" "ployz $version"
    git -C "$work" push origin HEAD
    rm -rf "$work"
}

promote_published_release() {
    local tag=$1 channels checksums_dir newest_stable=
    channels=$(mktemp -d)
    push_channel_files "$channels" "$tag"
    dispatch_ployz_sh_site
    # Homebrew follows the unscoped stable pointer.
    [ -f "$channels/stable" ] && newest_stable=$(tr -d '[:space:]' < "$channels/stable")
    rm -rf "$channels"
    if [ "$newest_stable" != "$tag" ]; then
        return 0
    fi
    checksums_dir=$(mktemp -d)
    gh release download "$tag" --pattern checksums.txt --dir "$checksums_dir"
    push_homebrew_tap "$tag" "$checksums_dir/checksums.txt"
    rm -rf "$checksums_dir"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    tag=${1:-}
    [ -n "$tag" ] || {
        echo "usage: $0 <tag>" >&2
        exit 1
    }
    promote_published_release "$tag"
fi
