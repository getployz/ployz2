#!/usr/bin/env bash
set -euo pipefail

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
: "${PLOYZ_QUALIFICATION_PHASE:?}"
: "${PLOYZ_QUALIFICATION_STATE:?}"

out=$(mktemp -d "$ROOT/cloud/.qualification-883.XXXXXX")
trap 'rm -rf -- "$out"' EXIT
PLOYZ_QUALIFICATION_DRIVER_OUT="$out" \
  npm exec --yes --package=pnpm@11.7.0 -- pnpm --dir "$ROOT/cloud" exec vite build \
  --config vite.qualification.config.ts
node "$out/tailcat-883.live.js"
