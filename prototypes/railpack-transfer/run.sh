#!/usr/bin/env bash
# THROWAWAY infrastructure experiment. No production runner implementation.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$PWD
mkdir -p "$ROOT/.scratch"
OUT=$(mktemp -d "$ROOT/.scratch/railpack-transfer-run.XXXXXX")
exec > >(tee "$OUT/transcript.log") 2>&1
printf 'Evidence: %s\n' "$OUT"
ID="railpack-proto-$(date +%s)-$$"
TESTKIT=ghcr.io/getployz/ployz2-testkit@sha256:ed06389be0692b2dedc93c39fcf91f61592ba4f3e3b121f665817a63ae8c2212
BUILDKIT=moby/buildkit@sha256:de10faf919fc71ba4eb1dd7bd6449566d012b0c9436b1c61bfee21d621b009aa
FRONTEND=ghcr.io/railwayapp/railpack-frontend@sha256:db24dc37640b6887c3d455b40876ea30f75182964479670cba6e4cde7ffef103
BINFMT=tonistiigi/binfmt@sha256:400a4873b838d1b89194d982c45e5fb3cda4593fbfd7e08a02e76b03b21166f0
IMAGE=railpack-prototype.invalid/app:multi
APP="$ROOT/prototypes/railpack-transfer/app"
cleanup() {
    docker buildx rm "$ID" >/dev/null 2>&1 || true
    docker volume rm "buildx_buildkit_${ID}0_state" >/dev/null 2>&1 || true
    for n in 0 1 2; do docker rm -fv "$ID-$n" >/dev/null 2>&1 || true; done
    docker network rm "$ID" >/dev/null 2>&1 || true
    printf 'Evidence retained: %s\n' "$OUT"
}
trap cleanup EXIT
for cmd in docker curl jq python3; do command -v "$cmd" >/dev/null; done
[[ $(uname -m) == x86_64 ]]
[[ $(docker version --format '{{.Server.Version}}') == 29.1.3 ]]
[[ $(docker buildx version) == *' 0.30.1 '* ]]
docker version > "$OUT/host-docker.txt"
docker buildx version > "$OUT/buildx.txt"
# QEMU must have F (fix-binary) to work inside nested containers. This installs
# only ARM64 support on the dedicated development VM; registration remains.
if ! grep -q 'flags:.*F' /proc/sys/fs/binfmt_misc/qemu-aarch64 2>/dev/null; then
    docker run --privileged --rm "$BINFMT" --uninstall qemu-aarch64 || true
    docker run --privileged --rm "$BINFMT" --install arm64
fi
cat /proc/sys/fs/binfmt_misc/qemu-aarch64 > "$OUT/binfmt.txt"
docker run --rm "$BINFMT" --version > "$OUT/qemu-version.txt" 2>&1
curl -fsSL https://github.com/railwayapp/railpack/releases/download/v0.39.0/railpack-v0.39.0-x86_64-unknown-linux-musl.tar.gz -o "$OUT/railpack.tar.gz"
printf '728407f5cdb9e9bc1cdd07f568419344a20e71b0a5a9fd90a9cfbaca0a6c94f7  %s\n' "$OUT/railpack.tar.gz" | sha256sum -c -
tar -xzf "$OUT/railpack.tar.gz" -C "$OUT"
curl -fsSL https://github.com/regclient/regclient/releases/download/v0.11.6/regctl-linux-amd64 -o "$OUT/regctl"
printf '8e0e62a497fcdb8048d18aa927a139613176ba0531f412bc541044e28f9856bd  %s\n' "$OUT/regctl" | sha256sum -c -
chmod +x "$OUT/regctl"
"$OUT/railpack" --version > "$OUT/railpack-version.txt"
"$OUT/regctl" version > "$OUT/regctl-version.txt"
"$OUT/railpack" prepare "$APP" --plan-out "$OUT/plan.json" --info-out "$OUT/info.json" > "$OUT/prepare.log" 2>&1
builder() { docker buildx create --name "$ID" --driver docker-container --driver-opt "image=$BUILDKIT" --bootstrap; }
build() {
    docker buildx build --builder "$ID" --platform "$1" \
        --build-arg "BUILDKIT_SYNTAX=$FRONTEND" -f "$OUT/plan.json" \
        --output "type=oci,dest=$OUT/$2.tar" --provenance=false --progress plain "$APP"
}
builder > "$OUT/builder.log" 2>&1
docker buildx inspect "$ID" > "$OUT/buildkit-version.txt"
if build linux/amd64,linux/arm64 rejected > "$OUT/multi-invocation.log" 2>&1; then
    echo 'Unexpected frontend multi-platform support; update this experiment.'; exit 1
fi
grep -q 'multiple platforms are not supported' "$OUT/multi-invocation.log"
for arch in amd64 arm64; do
    build "linux/$arch" "$arch" > "$OUT/build-$arch.log" 2>&1
    "$OUT/regctl" image import "ocidir://$OUT/layout:$arch" "$OUT/$arch.tar"
done
"$OUT/regctl" index create "ocidir://$OUT/layout:multi" \
    --ref "ocidir://$OUT/layout:amd64" --ref "ocidir://$OUT/layout:arm64"
"$OUT/regctl" manifest get "ocidir://$OUT/layout:multi" --format raw-body > "$OUT/index.json"
"$OUT/regctl" image export "ocidir://$OUT/layout:multi" "$OUT/multi.tar" --name "$IMAGE"
echo 'PASS: two Railpack outputs assembled into one OCI index'
docker inspect "buildx_buildkit_${ID}0" --format '{{.Id}} {{json .Mounts}}' > "$OUT/cache-before.txt"
docker buildx rm --keep-state "$ID"
builder > "$OUT/recreate.log" 2>&1
docker inspect "buildx_buildkit_${ID}0" --format '{{.Id}} {{json .Mounts}}' > "$OUT/cache-after.txt"
build linux/amd64 cached > "$OUT/cache-build.log" 2>&1
read -r before_id before_mounts < "$OUT/cache-before.txt"
read -r after_id after_mounts < "$OUT/cache-after.txt"
[[ $before_id != "$after_id" && $before_mounts == "$after_mounts" ]]
awk '/^#[0-9]+ npm install$/ {id=$1} $1==id && $2=="CACHED" {ok=1} END {exit !ok}' "$OUT/cache-build.log"
echo 'PASS: recreated builder reused retained cache (see per-step log)'
docker network create "$ID" >/dev/null
for n in 0 1 2; do
    # Both peers are AMD64 hardware. Receiver's Docker CLI default models an
    # ARM64 destination for the existing peer-pull subprocess, explicitly.
    env_args=()
    if [[ $n == 2 ]]; then env_args=(-e DOCKER_DEFAULT_PLATFORM=linux/arm64); fi
    docker run -d --privileged --name "$ID-$n" --network "$ID" "${env_args[@]}" "$TESTKIT" >/dev/null
    timeout 120 bash -c 'until docker exec "$1" test -S /run/ployz/ployz.sock; do sleep 1; done' _ "$ID-$n"
done
for n in 0 1 2; do
    docker exec "$ID-$n" docker version > "$OUT/docker-$n.txt"
    docker exec "$ID-$n" docker info --format '{{json .DriverStatus}}' > "$OUT/store-driver-$n.json"
done
docker exec "$ID-0" ployz version > "$OUT/ployz-version.txt"
docker exec "$ID-0" sha256sum /usr/local/bin/ployz /usr/local/bin/ployzd > "$OUT/ployz-binaries.txt"
docker exec "$ID-0" ployz machine init --name builder --no-install --no-dns --no-ingress \
    --storage none --public-ip none --yes > "$OUT/init.log" 2>&1
docker exec "$ID-0" cat /root/.ssh/id_ed25519.pub > "$OUT/key.pub"
for n in 1 2; do
    docker cp "$OUT/key.pub" "$ID-$n:/root/.ssh/authorized_keys"
    docker exec "$ID-$n" chown root:root /root/.ssh/authorized_keys
    docker exec "$ID-$n" chmod 600 /root/.ssh/authorized_keys
    addr=$(docker inspect --format '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$ID-$n")
    docker exec "$ID-0" ployz machine add "root@$addr" --name "peer$n" --no-install \
        --no-ingress --storage none --public-ip none --wg-endpoint "$addr:51820" --yes > "$OUT/join-$n.log" 2>&1
done
docker exec "$ID-0" ployz machine ls > "$OUT/machines.txt"
docker exec -i "$ID-0" docker load < "$OUT/multi.tar" > "$OUT/load.log"
for n in 1 2; do
    if docker exec "$ID-$n" docker image inspect "$IMAGE" > "$OUT/empty-$n.log" 2>&1; then
        echo 'Destination unexpectedly already has image'; exit 1
    fi
done
python3 prototypes/railpack-transfer/check-store.py "$OUT/layout" "$ID-0" amd64 arm64 > "$OUT/content-0.json"
docker exec "$ID-0" ployz image push --machine peer1 --machine peer2 "$IMAGE" > "$OUT/push.log" 2>&1
python3 prototypes/railpack-transfer/check-store.py "$OUT/layout" "$ID-1" amd64 arm64 > "$OUT/content-1.json"
python3 prototypes/railpack-transfer/check-store.py "$OUT/layout" "$ID-2" arm64 > "$OUT/content-2.json"
jq -e '.platforms.amd64.complete == false' "$OUT/content-2.json" >/dev/null
for n in 0 1 2; do
    docker exec "$ID-$n" docker image ls --tree "$IMAGE" > "$OUT/tree-$n.txt" 2>&1
    docker exec "$ID-$n" docker image inspect "$IMAGE" > "$OUT/inspect-$n.json"
done
# Only first target starts the serving helper: second receives via peer pull.
docker exec "$ID-1" docker inspect ployz-unregistry > "$OUT/helper.json"
if docker exec "$ID-2" docker inspect ployz-unregistry > "$OUT/no-helper.log" 2>&1; then
    echo 'Second target unexpectedly opened ingest'; exit 1
fi
for pair in '1 amd64 x64' '1 arm64 arm64' '2 arm64 arm64'; do
    read -r n arch node_arch <<< "$pair"
    docker exec "$ID-$n" docker run --rm --pull never --platform "linux/$arch" "$IMAGE" > "$OUT/run-$n-$arch.log" 2>&1
    grep -q "\"arch\":\"$node_arch\"" "$OUT/run-$n-$arch.log"
done
docker exec "$ID-1" docker logs ployz-unregistry > "$OUT/transfer-http.log" 2>&1
echo 'PASS: first hop contains both variants; peer hop selects ARM64; all three runs match'
