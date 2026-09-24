#!/usr/bin/env bash
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/dist" "$TMP/bin"
for binary in ployz ployzd ployz-uninstall; do
    printf '#!/bin/sh\nexit 0\n' > "$TMP/bin/$binary"
    chmod 0755 "$TMP/bin/$binary"
done
for platform in linux_amd64 linux_arm64 macos_amd64 macos_arm64; do
    tar -czf "$TMP/dist/ployz_$platform.tar.gz" -C "$TMP/bin" ployz
done
for arch in amd64 arm64; do
    tar -czf "$TMP/dist/ployzd_linux_$arch.tar.gz" -C "$TMP/bin" ployzd ployz-uninstall
done
bash "$ROOT/scripts/pack-release.sh" "$TMP/dist"
DIST="$TMP/dist" bash "$ROOT/scripts/verify-release.sh" artifacts
# Archive members are exact: the CLI ships alone, the daemon ships with its uninstaller.
for archive in ployz_linux_amd64:ployz,ployz-uninstall ployzd_linux_amd64:ployzd; do
    members=${archive#*:}
    archive=${archive%%:*}
    cp "$TMP/dist/$archive.tar.gz" "$TMP/original.tar.gz"
    tar -czf "$TMP/dist/$archive.tar.gz" -C "$TMP/bin" ${members//,/ }
    if DIST="$TMP/dist" bash "$ROOT/scripts/verify-release.sh" artifacts > "$TMP/error" 2>&1; then
        echo "accepted $archive with members $members" >&2
        exit 1
    fi
    grep -Fq 'members are' "$TMP/error"
    mv "$TMP/original.tar.gz" "$TMP/dist/$archive.tar.gz"
done
# Publication must accept exactly these six archives and stop before gh on extras.
cat > "$TMP/bin/gh" <<'SH'
#!/bin/sh
printf '%s\n' "$*" >> "$GH_CALLS"
SH
chmod 0755 "$TMP/bin/gh"
export GH_CALLS="$TMP/gh-calls"
PATH="$TMP/bin:$PATH" DIST="$TMP/dist" bash "$ROOT/scripts/publish-github-release.sh" v1.2.3
grep -Fq 'release create v1.2.3 --draft' "$GH_CALLS"
rm "$GH_CALLS"
touch "$TMP/dist/retired_linux_amd64.tar.gz"
if DIST="$TMP/dist" bash "$ROOT/scripts/verify-release.sh" artifacts > "$TMP/error" 2>&1; then
    echo 'accepted an extra release archive' >&2
    exit 1
fi
grep -Fq 'archive set differs' "$TMP/error"
if PATH="$TMP/bin:$PATH" DIST="$TMP/dist" bash "$ROOT/scripts/publish-github-release.sh" v1.2.3 > "$TMP/error" 2>&1; then
    echo 'published an extra release archive' >&2
    exit 1
fi
grep -Fq 'release asset set differs' "$TMP/error"
[ ! -e "$GH_CALLS" ]
# Channel pointers only move forward: beta follows a newer stable, an older-line fix moves only
# its own line, and the ployz.sh site serves both the line and unscoped pointers.
(
    # shellcheck source=scripts/promote-release.sh
    source "$ROOT/scripts/promote-release.sh"
    channels="$TMP/channels"
    publish() {
        STABLE_ADVANCED=
        write_channel_files "$channels" "$1"
    }
    expect() {
        local dir=$1
        shift
        while [ "$#" -gt 0 ]; do
            if [ "$(cat "$dir/$1" 2>/dev/null)" != "$2" ]; then
                echo "pointer $1 is '$(cat "$dir/$1" 2>/dev/null)', expected '$2'" >&2
                exit 1
            fi
            shift 2
        done
    }
    publish v0.2.0-beta.1
    expect "$channels" v0/beta v0.2.0-beta.1 beta v0.2.0-beta.1 v0/stable '' stable ''
    publish v0.2.0
    expect "$channels" v0/stable v0.2.0 v0/beta v0.2.0 stable v0.2.0 beta v0.2.0
    [ -n "$STABLE_ADVANCED" ]
    publish v0.2.1-beta.2
    publish v0.2.1-beta.10
    publish v0.2.0
    expect "$channels" v0/stable v0.2.0 v0/beta v0.2.1-beta.10 beta v0.2.1-beta.10
    [ -z "$STABLE_ADVANCED" ]
    publish v1.0.0
    publish v0.2.10
    expect "$channels" v0/stable v0.2.10 v0/beta v0.2.10 v1/stable v1.0.0 stable v1.0.0 beta v1.0.0
    [ -z "$STABLE_ADVANCED" ]
    PLOYZ_SH_SITE_DIR="$TMP/site" PLOYZ_SH_CHANNELS_DIR="$channels" \
        bash "$ROOT/scripts/stage-ployz-sh-site.sh" > /dev/null
    expect "$TMP/site" v0/stable v0.2.10 v1/beta v1.0.0 stable v1.0.0
)
echo 'release packaging contracts passed'
