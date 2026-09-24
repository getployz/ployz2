#!/usr/bin/env bash

set -euo pipefail

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
# shellcheck source=scripts/homebrew-formula.sh
source "$ROOT/scripts/homebrew-formula.sh"
# shellcheck source=scripts/release-tag.sh
source "$ROOT/scripts/release-tag.sh"

# Set when the unscoped stable pointer moved, which is what Homebrew follows.
STABLE_ADVANCED=

# Moves pointer file $2 under $1 to tag $3 only when the tag is higher, so an older-line fix
# never moves a channel backwards.
advance_pointer() {
    local dest_dir=$1 pointer=$2 tag=$3 current=
    [ -f "$dest_dir/$pointer" ] && current=$(tr -d '[:space:]' < "$dest_dir/$pointer")
    if [ -n "$current" ] && ! release_tag_higher "$tag" "$current"; then
        return 1
    fi
    mkdir -p "$(dirname "$dest_dir/$pointer")"
    printf '%s\n' "$tag" > "$dest_dir/$pointer"
}

# Daemons read their line's pointer (v0/stable); only install.sh reads the unscoped one.
# beta is the highest release, so a stable tag advances it too.
write_channel_files() {
    local dest_dir=$1 tag=$2 line channel channels
    line=$(release_line_for_tag "$tag")
    channel=$(channel_name_for_tag "$tag")
    channels=beta
    [ "$channel" = stable ] && channels="stable beta"
    for channel in $channels; do
        advance_pointer "$dest_dir" "$line/$channel" "$tag" || true
        if advance_pointer "$dest_dir" "$channel" "$tag" && [ "$channel" = stable ]; then
            STABLE_ADVANCED=1
        fi
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
    rm -rf "$work"
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
    local tag=$1 checksums_dir
    push_channel_file "$tag"
    dispatch_ployz_sh_site
    if [ -z "$STABLE_ADVANCED" ]; then
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
