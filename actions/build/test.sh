#!/usr/bin/env bash
# Runs prepare.sh and build.sh against stubbed curl, docker, sudo and ployz.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/stubs" "$tmp/runner" "$tmp/workspace"

commit=0123456789abcdef0123456789abcdef01234567
fingerprint=$(printf 'f%.0s' {1..64})
cat >"$tmp/check-in.json" <<JSON
{"grant":"ployzgrant1:grant-secret","commit":"$commit","fingerprint":"$fingerprint",
 "deployment":{"projectName":"p","snapshots":[{"config":{},"resolvedEnv":{"TOKEN":"s3cr3t","KEY":"line-one\nline-two"}}]}}
JSON

cat >"$tmp/stubs/curl" <<STUB
#!/usr/bin/env bash
out=; url=
while [ \$# -gt 0 ]; do
  case "\$1" in -o) out=\$2; shift ;; -H|-X|--retry) shift ;; -*) ;; *) url=\$1 ;; esac
  shift
done
case "\$url" in
  https://ployz.sh) printf '%s\n' 'mkdir -p "\$INSTALL_BIN_DIR"; : > "\$INSTALL_BIN_DIR/ployz"; chmod +x "\$INSTALL_BIN_DIR/ployz"' > "\$out" ;;
  *audience=https%3A%2F%2Fcloud.test) echo '{"value":"oidc-token"}' ;;
  https://cloud.test/api/builds/b-1/check-in) cp "$tmp/check-in.json" "\$out" ;;
  *) echo "unexpected curl \$url" >&2; exit 1 ;;
esac
STUB
cat >"$tmp/stubs/docker" <<'STUB'
#!/usr/bin/env bash
echo '[["driver-type","io.containerd.snapshotter.v1"]]'
STUB
cat >"$tmp/stubs/ployz" <<STUB
#!/usr/bin/env bash
[ "\$PLOYZ_BUILD_GRANT" = ployzgrant1:grant-secret ] || { echo "grant not exported" >&2; exit 1; }
[ "\$*" = "build --deployment $tmp/runner/ployz-build/deployment.json --commit $commit --fingerprint $fingerprint --source $tmp/workspace" ] || { echo "bad args: \$*" >&2; exit 1; }
echo '{"digest":"sha256:abc","tag":"ployz-sha256-abc","platforms":["linux/amd64"]}'
STUB
chmod +x "$tmp/stubs/"*

export PATH="$tmp/stubs:$PATH" RUNNER_TEMP="$tmp/runner" GITHUB_WORKSPACE="$tmp/workspace"
export GITHUB_OUTPUT="$tmp/output" GITHUB_PATH="$tmp/path" GITHUB_STEP_SUMMARY="$tmp/summary"
export ACTIONS_ID_TOKEN_REQUEST_URL="https://token.test/?x=1" ACTIONS_ID_TOKEN_REQUEST_TOKEN=request-token
export PLOYZ_BUILD_ID=b-1 PLOYZ_CLOUD=https://cloud.test/ PLOYZ_VERSION=0.1.0-beta.28

log=$("$here/prepare.sh")
for mask in oidc-token ployzgrant1:grant-secret s3cr3t line-one line-two; do
  grep -qxF "::add-mask::$mask" <<<"$log" || { echo "FAIL: $mask not masked" >&2; exit 1; }
done
grep -qxF "commit=$commit" "$GITHUB_OUTPUT" || { echo "FAIL: commit output" >&2; exit 1; }
jq -e '.snapshots[0].resolvedEnv.TOKEN == "s3cr3t"' "$RUNNER_TEMP/ployz-build/deployment.json" >/dev/null
[ -x "$(cat "$GITHUB_PATH")/ployz" ] || { echo "FAIL: ployz not installed" >&2; exit 1; }

"$here/build.sh"
grep -qxF "digest=sha256:abc" "$GITHUB_OUTPUT" || { echo "FAIL: digest output" >&2; exit 1; }

if PLOYZ_VERSION=latest "$here/prepare.sh" >/dev/null 2>&1; then echo "FAIL: accepted a floating version" >&2; exit 1; fi
if PLOYZ_CLOUD=https://evil.test/path "$here/prepare.sh" >/dev/null 2>&1; then echo "FAIL: accepted a Cloud URL with a path" >&2; exit 1; fi
echo "ok"
