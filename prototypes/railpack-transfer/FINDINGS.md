# Railpack → Direct Image Transfer: prototype findings

**Verdict: feasible with an explicit index-assembly step.** Railpack 0.39.0
rejects a multi-platform frontend invocation. Two single-platform Railpack builds,
assembled locally with regctl, produced one OCI index. Ployz transferred both
variants to its first target, then transferred the selected ARM64 variant to a
second target. The application ran as AMD64 natively and ARM64 under QEMU.

Run from the repository root on this dedicated AMD64 Linux Docker VM:

```bash
prototypes/railpack-transfer/run.sh
```

The harness takes about 4 minutes with upstream downloads available. It requires
Docker Engine 29.1.3, buildx 0.30.1, curl, jq, and Python 3. It downloads checksummed
Railpack/regctl tools into `.scratch`, creates three privileged disposable testkit
Machines and a disposable BuildKit builder, and removes its own containers,
network, and builder cache on exit. Logs and OCI archives remain in the printed
`.scratch/railpack-transfer-run.*` directory. No application Machines, shared
images, or shared volumes are pruned. If ARM64 binfmt lacks the `F` flag, it
replaces that registration using pinned tonistiigi/binfmt; this host registration
remains after the run. Existing compatible ARM64 registration is reused.

The tracked [evidence](evidence/) is a text-only snapshot of the final run on
2026-09-08. Log whitespace is normalized; manifest JSON bytes are preserved.
Image archives, downloaded executables, and SSH private keys are not
committed. All prototype files stay on `feat/railpack-transfer-prototype`.

| Stage | Result | Evidence |
|---|---|---|
| Real Railpack detection | PASS: Node, pinned Node 22.16.0, `npm run start` | [prepare](evidence/prepare.log), [plan](evidence/plan.json) |
| One frontend invocation for both platforms | FAIL: `multiple platforms are not supported` | [failure](evidence/multi-invocation.log) |
| Separate builds → one locally assembled index | PASS: distinct AMD64 and ARM64 manifests | [index](evidence/index.json), [AMD64 build](evidence/build-amd64.log), [ARM64 build](evidence/build-arm64.log) |
| Docker containerd store | PASS: index plus both manifests, configs, and every layer present | [builder contents](evidence/content-0.json) |
| Ployz first target | PASS: preserves the same index and all content for both platforms | [push](evidence/push.log), [first-target contents](evidence/content-1.json) |
| Ployz Machine-to-Machine hop | PASS under explicit ARM64 selection: ARM64 content complete; AMD64 content absent | [second-target contents](evidence/content-2.json), [HTTP transfer log](evidence/transfer-http.log) |
| Application execution, with `--pull never` | PASS: first target prints `x64` and `arm64`; second target prints `arm64` | [native](evidence/run-1-amd64.log), [emulated first hop](evidence/run-1-arm64.log), [emulated peer hop](evidence/run-2-arm64.log) |
| Builder removal/recreation with cache | PASS: different container IDs, same state volume, `npm install` is `CACHED` | [before](evidence/cache-before.txt), [after](evidence/cache-after.txt), [cache build](evidence/cache-build.log) |
| Native ARM64 Machine | NOT TESTED: this VM and all three nested Machines are AMD64 | [Machines](evidence/machines.txt) |

## What actually ran

```text
Railpack prepare → frontend(amd64) + frontend(arm64)
  → regctl local OCI index → Docker load on builder Machine
  → ployz image push --machine peer1 --machine peer2 IMAGE
      → peer1: complete index, both variants
      → peer2: same index, ARM64 content only
```

The [shell harness](run.sh) contains the exact commands and pinned image
references. The essential build/assembly commands are:

```bash
docker buildx build --platform linux/amd64 --build-arg BUILDKIT_SYNTAX="$FRONTEND" \
  -f "$OUT/plan.json" --output "type=oci,dest=$OUT/amd64.tar" --provenance=false "$APP"
# Repeat for linux/arm64 into arm64.tar, importing each tar to a separate local tag.
regctl index create "ocidir://$OUT/layout:multi" \
  --ref "ocidir://$OUT/layout:amd64" --ref "ocidir://$OUT/layout:arm64"
regctl image export "ocidir://$OUT/layout:multi" "$OUT/multi.tar" --name "$IMAGE"
```

The actual command includes the isolated `--builder` and records plain progress.
Railpack and BuildKit run on the host; the archive is then loaded into nested
Machine 0 for transfer. This is not a remote-build dispatch implementation.
The builds do **not** overwrite the same Docker tag. regctl assembles the two
manifests in an OCI directory without publishing them to any registry. Ployz's
own unregistry 0.4.1 helper serves the resulting content directly from the first
Machine's containerd store. The second Machine does not start an ingest helper;
its downloads appear as GETs in the first Machine's HTTP log. Neither destination
had the sample image before transfer. The `.invalid` image name and runtime
`--pull never` prevent a registry download from substituting for transferred data.

[check-store.py](check-store.py) checks the index, manifest, config, and layer
hashes against the actual containerd content inventory before running containers.
This matters: Docker's image tree can list a platform whose content size is zero.
The second target retains the full index but lacks the unselected platform's
manifest/config/layers; an index descriptor is not proof of runnable content.

## Pinned versions

| Component | Version / immutable reference |
|---|---|
| Railpack CLI | 0.39.0; downloaded archive SHA-256 checked by harness |
| Railpack frontend | v0.39.0, `sha256:db24dc37640b6887c3d455b40876ea30f75182964479670cba6e4cde7ffef103` |
| BuildKit | v0.26.2, `sha256:de10faf919fc71ba4eb1dd7bd6449566d012b0c9436b1c61bfee21d621b009aa` |
| Docker Engine / CLI | 29.1.3 on host and testkit; buildx 0.30.1 on host |
| regctl | v0.11.6; executable SHA-256 checked by harness |
| Ployz CLI / daemon | 0.1.2-beta.35 from testkit digest `sha256:ed06389be0692b2dedc93c39fcf91f61592ba4f3e3b121f665817a63ae8c2212` |
| Transfer helper | `ghcr.io/psviderski/unregistry:0.4.1`, preloaded in the pinned testkit |
| ARM64 emulator used | QEMU v10.2.3, binfmt e29e7d7; installer digest `sha256:400a4873b838d1b89194d982c45e5fb3cda4593fbfd7e08a02e76b03b21166f0` |

Exact Docker component versions, binary hashes, builder version, and binfmt flags
are included alongside the logs. The testkit has no source-revision label;
this proves the pinned shipped binaries, not a fresh compilation of this branch.

## Smallest production implication and remaining gaps

**Build execution needs per-platform Railpack solves and OCI index assembly** for
this pinned version. No production Rust changes were needed to demonstrate
Direct Image Transfer from a source with complete content. Railpack's
[pinned frontend source](https://github.com/railwayapp/railpack/blob/v0.39.0/buildkit/frontend.go#L143)
explicitly refuses comma-separated platforms. The upstream
[prepare/frontend flow](https://railpack.com/platforms/running-railpack-in-production/)
and [regctl index creation](https://regclient.org/cli/regctl/index/create/)
provided the build and assembly primitives.

The peer test sets `DOCKER_DEFAULT_PLATFORM=linux/arm64` in the receiving Machine's
environment. Its existing Docker CLI peer-pull subprocess therefore selects
ARM64. Its daemon still reports AMD64 hardware. This proves transport and Docker
selection under emulation, **not** automatic selection on a native ARM64 Machine
or Ployz deployment placement. Repeat the same transfer on native ARM64 hardware
before claiming that coverage.

Keep the complete build Machine as the distribution source, or check actual
platform content before selecting a peer. The current tag-only peer lookup in
[image.rs](../../crates/ployz/src/image.rs) can see a tag on a Machine holding only
one variant; this experiment does not establish safe arbitrary relay chains.
It also does not validate every Railpack provider, attestations, cancellation,
concurrency, cache budgets, or independent npm cache-mount reuse. Retained build
layer cache is proven; byte-for-byte reproducible builds are not claimed.

Validation rung: **4, informing cluster experiment**, executed by `run.sh` with
real Docker, Ployz, WireGuard, image transfer, and application processes. `bash -n`
is the rung-1 syntax check. This is not rung-5 release Authority. The existing
`direct-image-transfer` entry in `evidence/product-paths.tsv` remains unchanged:
this prototype introduces no product behavior or ignored Rust test.
