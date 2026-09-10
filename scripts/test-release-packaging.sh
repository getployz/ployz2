#!/usr/bin/env bash
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/dist" "$TMP/bin"
for binary in ployz ployzd ployz-tailcat ployz-uninstall ployz-relay; do
    printf '#!/bin/sh\nexit 0\n' > "$TMP/bin/$binary"
    chmod 0755 "$TMP/bin/$binary"
done
for platform in linux_amd64 linux_arm64 macos_amd64 macos_arm64; do
    tar -czf "$TMP/dist/ployz_$platform.tar.gz" -C "$TMP/bin" ployz ployz-tailcat
done
for arch in amd64 arm64; do
    tar -czf "$TMP/dist/ployzd_linux_$arch.tar.gz" -C "$TMP/bin" ployzd ployz-tailcat ployz-uninstall
    tar -czf "$TMP/dist/ployz-relay_linux_$arch.tar.gz" -C "$TMP/bin" ployz-relay
done
bash "$ROOT/scripts/pack-release.sh" "$TMP/dist"
DIST="$TMP/dist" bash "$ROOT/scripts/verify-release.sh" artifacts
# Both products must ship the helper; checking one archive alone misses regressions.
for archive in ployz_linux_amd64 ployzd_linux_amd64; do
    cp "$TMP/dist/$archive.tar.gz" "$TMP/original.tar.gz"
    binary=${archive%%_*}
    tar -czf "$TMP/dist/$archive.tar.gz" -C "$TMP/bin" "$binary"
    if DIST="$TMP/dist" bash "$ROOT/scripts/verify-release.sh" artifacts > "$TMP/error" 2>&1; then
        echo "accepted $archive without its helper" >&2
        exit 1
    fi
    grep -Fq 'members are' "$TMP/error"
    mv "$TMP/original.tar.gz" "$TMP/dist/$archive.tar.gz"
done
echo 'release helper packaging contracts passed'
