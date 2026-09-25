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
  case "\$1" in -o) out=\$2; shift ;; --data-binary) cat "\${2#@}" >>"$tmp/posted.jsonl"; shift ;; -H|-X|--retry) shift ;; -*) ;; *) url=\$1 ;; esac
  shift
done
case "\$url" in
  https://ployz.sh) printf '%s\n' 'mkdir -p "\$INSTALL_BIN_DIR"; : > "\$INSTALL_BIN_DIR/ployz"; chmod +x "\$INSTALL_BIN_DIR/ployz"' > "\$out" ;;
  *audience=https%3A%2F%2Fcloud.test) echo '{"value":"oidc-token"}' ;;
  https://cloud.test/api/builds/b-1/check-in) cp "$tmp/check-in.json" "\$out" ;;
  https://cloud.test/api/builds/b-1/steps) ;;
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
[ "\$*" = "build --deployment $tmp/runner/ployz-build/deployment.json --commit $commit --fingerprint $fingerprint --source $tmp/workspace --events $tmp/runner/ployz-build/events.jsonl" ] || { echo "bad args: \$*" >&2; exit 1; }
events=$tmp/runner/ployz-build/events.jsonl
printf '%s\n' '{"at":1,"event":{"Build":{"Stage":"Building"}}}' '{"at":2,"event":{"Build":{"Stage":"Output"}}}' >>"\$events"
# Long enough for a report while the build runs.
sleep 0.5
echo '{"at":3,"event":{"Build":{"Stage":"Cleanup"}}}' >>"\$events"
[ -z "\${PLOYZ_FAIL:-}" ] || exit 3
echo '{"digest":"sha256:abc","tag":"ployz-sha256-abc","platforms":["linux/amd64"]}'
STUB
chmod +x "$tmp/stubs/"*

export PATH="$tmp/stubs:$PATH" RUNNER_TEMP="$tmp/runner" GITHUB_WORKSPACE="$tmp/workspace"
export GITHUB_OUTPUT="$tmp/output" GITHUB_PATH="$tmp/path" GITHUB_STEP_SUMMARY="$tmp/summary"
export ACTIONS_ID_TOKEN_REQUEST_URL="https://token.test/?x=1" ACTIONS_ID_TOKEN_REQUEST_TOKEN=request-token
export PLOYZ_BUILD_ID=b-1 PLOYZ_CLOUD=https://cloud.test/ PLOYZ_VERSION=0.1.0-beta.28 PLOYZ_STEPS_INTERVAL=0.1

log=$("$here/prepare.sh")
for mask in oidc-token ployzgrant1:grant-secret s3cr3t line-one line-two; do
  grep -qxF "::add-mask::$mask" <<<"$log" || { echo "FAIL: $mask not masked" >&2; exit 1; }
done
grep -qxF "commit=$commit" "$GITHUB_OUTPUT" || { echo "FAIL: commit output" >&2; exit 1; }
jq -e '.snapshots[0].resolvedEnv.TOKEN == "s3cr3t"' "$RUNNER_TEMP/ployz-build/deployment.json" >/dev/null
[ -x "$(cat "$GITHUB_PATH")/ployz" ] || { echo "FAIL: ployz not installed" >&2; exit 1; }

"$here/build.sh" >/dev/null
grep -qxF "digest=sha256:abc" "$GITHUB_OUTPUT" || { echo "FAIL: digest output" >&2; exit 1; }
# Steps go to Cloud while the build runs, each batch from where the last one ended; the last carries the platforms.
# shellcheck disable=SC2016 # jq variables, not shell ones
reported='length >= 2 and .[0].from == 0 and (.[0] | has("platforms") | not)
  and ([.[].events[].at] == [1, 2, 3]) and (. as $b | all(range(1; $b | length); $b[.].from == $b[. - 1].from + ($b[. - 1].events | length)))'
jq -s -e "$reported"' and .[-1].platforms == ["linux/amd64"]' "$tmp/posted.jsonl" >/dev/null ||
  { echo "FAIL: Build Steps not posted as they happened" >&2; cat "$tmp/posted.jsonl" >&2; exit 1; }
# A failed build still reports its steps, then fails the job.
rm "$tmp/posted.jsonl"
if PLOYZ_FAIL=1 "$here/build.sh" >/dev/null 2>&1; then echo "FAIL: a failed build passed" >&2; exit 1; fi
jq -s -e "$reported"' and .[-1].platforms == []' "$tmp/posted.jsonl" >/dev/null || { echo "FAIL: a failed build did not report" >&2; exit 1; }

if PLOYZ_VERSION=latest "$here/prepare.sh" >/dev/null 2>&1; then echo "FAIL: accepted a floating version" >&2; exit 1; fi
if PLOYZ_CLOUD=https://evil.test/path "$here/prepare.sh" >/dev/null 2>&1; then echo "FAIL: accepted a Cloud URL with a path" >&2; exit 1; fi
echo "ok"
