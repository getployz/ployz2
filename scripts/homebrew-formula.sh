#!/usr/bin/env bash

set -euo pipefail

checksum_for() {
    local checksums=$1 archive=$2 hash
    hash=$(awk -v archive="$archive" '$2 == archive { print $1; exit }' "$checksums")
    [ -n "$hash" ] || {
        echo "checksums.txt has no hash for $archive" >&2
        return 1
    }
    printf '%s\n' "$hash"
}

write_homebrew_formula() {
    local dest=$1 version=$2 tag=$3 repo=$4
    local hash_linux_amd64=$5 hash_linux_arm64=$6 hash_macos_amd64=$7 hash_macos_arm64=$8
    mkdir -p "$(dirname "$dest")"
    cat > "$dest" <<EOF
# typed: false
# frozen_string_literal: true

class Ployz < Formula
  desc "Ployz CLI"
  homepage "https://github.com/${repo}"
  version "$version"

  on_macos do
    if Hardware::CPU.intel?
      url "https://github.com/${repo}/releases/download/${tag}/ployz_macos_amd64.tar.gz"
      sha256 "$hash_macos_amd64"

      def install
        bin.install "ployz"
      end
    end
    if Hardware::CPU.arm?
      url "https://github.com/${repo}/releases/download/${tag}/ployz_macos_arm64.tar.gz"
      sha256 "$hash_macos_arm64"

      def install
        bin.install "ployz"
      end
    end
  end

  on_linux do
    if Hardware::CPU.intel?
      if Hardware::CPU.is_64_bit?
        url "https://github.com/${repo}/releases/download/${tag}/ployz_linux_amd64.tar.gz"
        sha256 "$hash_linux_amd64"

        def install
          bin.install "ployz"
        end
      end
    end
    if Hardware::CPU.arm?
      if Hardware::CPU.is_64_bit?
        url "https://github.com/${repo}/releases/download/${tag}/ployz_linux_arm64.tar.gz"
        sha256 "$hash_linux_arm64"

        def install
          bin.install "ployz"
        end
      end
    end
  end

  def caveats
    <<~EOS
      This formula replaces the older getployz/ployz implementation as a clean break
      with manual transition, not an in-place compatibility promise.
    EOS
  end
end
EOF
}

write_homebrew_formula_from_checksums() {
    local checksums=$1 dest=$2 version=$3 tag=$4 repo=$5
    write_homebrew_formula "$dest" "$version" "$tag" "$repo" \
        "$(checksum_for "$checksums" ployz_linux_amd64.tar.gz)" \
        "$(checksum_for "$checksums" ployz_linux_arm64.tar.gz)" \
        "$(checksum_for "$checksums" ployz_macos_amd64.tar.gz)" \
        "$(checksum_for "$checksums" ployz_macos_arm64.tar.gz)"
}
