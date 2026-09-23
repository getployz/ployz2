# Uncloud-derived content: repository-history audit

Date: 2026-09-23. Resolves [Audit repository history for Uncloud-derived file contents](https://github.com/getployz/ployz2/issues/979), part of [Ployz 1.0 release — wayfinder map](https://github.com/getployz/ployz2/issues/973). Execution belongs to [Cut over to getployz/ployz](https://github.com/getployz/ployz2/issues/981).

**Finding:** deleting the old reconstruction inventories did not remove copied bytes from history. There are **71 confirmed copied upstream assets**: 58 generated CLI pages and 13 test fixtures. Three fixtures still survive byte-for-byte in the audited HEAD. A custom DNS diagnostic also survives with explicit copying evidence in its commit message. Six historical Corrosion schemas, including HEAD, retain near-verbatim upstream table definitions and need independent rewriting. No whole-file source-code copies were found by the normalized source comparison; that result does not establish independent authorship of translated code.

This audit recommends content actions. It does not reopen the map's attribution decision, grant legal clearance, or execute history rewriting.

## Frozen scope and evidence

| Item | Audited value |
|---|---|
| Ployz HEAD | [`6c21d8b1f97ac8175eba02bb9d1b4eb901571f34`](https://github.com/getployz/ployz2/tree/6c21d8b1f97ac8175eba02bb9d1b4eb901571f34) |
| Local revisions | 2,963 commits reachable from the captured refs; repository is not shallow |
| Local objects | 32,115 objects, including 13,795 distinct blobs (verified against the frozen ref OIDs even after other sessions advanced shared refs) |
| Ref coverage | Fetched all advertised origin heads/tags and GitHub PR head/merge refs; included existing local heads, remote-tracking refs, tags, and T3 checkpoints. Exact names and OIDs: [refs.txt](uncloud-history/refs.txt) |
| Upstream reconstruction baseline | [`psviderski/uncloud@b7e224a1eff98813b1d1a32034d977be24be994e`](https://github.com/psviderski/uncloud/tree/b7e224a1eff98813b1d1a32034d977be24be994e), explicitly pinned by the original reconstruction research |
| Upstream comparison | Full advertised mirror: 17,209 objects / 5,953 blobs; HEAD [`f68c9859d922c53d0bc4442958c581eb0713e5f7`](https://github.com/psviderski/uncloud/tree/f68c9859d922c53d0bc4442958c581eb0713e5f7). Exact shared assets verified again against the frozen baseline |
| Identifier results | Case-insensitive `uncloud\|uncld\|psviderski\|ucind\|ucnet`: 859 distinct matching blobs at 193 historical paths; 12 matching paths / 17 lines in HEAD |
| Pickaxe cross-check | `-G`: 618 matching commits; regex `-S`: 542 matching commits; both produce the same 193-path set as the complete blob scan |

Every distinct reachable blob was read, including files deleted from HEAD. Re-reading a blob once per commit would not reveal additional contents. Merge-parent raw diffs supplied every historical path alias, so a `rev-list --objects` representative filename was not mistaken for the only filename. Source comparisons also examine identifier-free candidates.

This is exhaustive for the **captured reachable revisions**, not deleted/unadvertised GitHub objects, inaccessible forks, reflog-only objects, or future commits. GitHub PR refs and T3 checkpoints are audited evidence, not an instruction to publish those refs. Rerun the audit on the cutover's exact selected heads and tags after cleanup; retaining old branches can retain old contents even when main is clean.

## Content actions

| Finding | Survives HEAD? | Recommendation |
|---|---|---|
| 58 generated CLI pages, originally `evidence/upstream/cli-reference/uc*.md`, later `evidence/cli-freeze/uc*.md` | No | **Drop from history** at both path families. These are confirmed copies, not merely similarly named commands. |
| 13 upstream e2e fixture files in `evidence/upstream/e2e-fixtures/` | Three survive at new paths below | **Drop from history** at the original prefix and every relocated alias. Remove current copied fixtures with the planned Compose deletion; if still needed, author replacement cases from Ployz requirements and retain only their new blobs. |
| `core/crates/ployz/tests/fixtures/compose-build-basic/compose.yaml` | Yes, blob `b99dd7c48999635fc58d95121ce1b002304314fc` | Rewrite/remove current fixture; purge copied historical versions. Earlier aliases: `ployz/tests/fixtures/compose-build-basic/compose.yaml`, `crates/ployz/tests/fixtures/compose-build-basic/compose.yaml`, and the evidence prefix. |
| Same directory's `service-first-dir/Dockerfile` | Yes, blob `9dae11481ac7626c3665e6532cb3eaf50a820ece` | Same treatment; all four aliases are in [exact matches](uncloud-history/exact-matches.md). This file is very small; the finding is confirmed import provenance, not a claim that a standard Docker instruction is uniquely authored. |
| Same directory's `service-second-dir/Dockerfile.alt` | Yes, blob `ea5543a9b90bad9c059e423c063f37c2dee190fe` | Same treatment. |
| Corrosion `schema.sql`: six historical blob versions, across the daemon path moves | Yes: `core/crates/ployzd/src/corrosion/schema.sql` | **Rewrite in place** from the Ployz schema contract, together with the already-decided removal of timestamps. The `cluster` and `machines` DDL closely reproduces upstream after removing comments; oldest `containers`/index definitions also track upstream. Replace the six historical schema blobs with contract-equivalent independently specified DDL, preserving each revision’s fields; do not erase the daemon or dashboard migrations. See the source manifest for exact IDs and aliases. |
| Custom hostname-expansion diagnostic in `core/crates/ployz/src/dns.rs` | Yes, `DomainRequired`, line 50 | **Rewrite in place** or remove with the planned DNS deletion. [Commit evidence](https://github.com/getployz/ployz2/commit/41b28f2577a6bec2240d4dc00586e9bdac82c541) explicitly says it preserved the upstream message; [upstream port implementation](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e/pkg/api/port.go) is the comparison source. Replace the historical diagnostic text narrowly, including old path aliases; do not drop the DNS module. |
| Public-IP discovery policy: six candidate historical blobs | Yes: `core/crates/ployzd/src/network/endpoints.rs` | **Conservative narrow rewrite** from independently stated discovery requirements. Same ordered provider triple, five-second timeout and fallback structure are a functional-port signal, not proof of verbatim code copying. Preserve automatic/disabled/override modes; replace only the policy portions in affected historical versions. |
| Image proxy setup: twelve candidate historical blobs | Yes: `core/crates/ployz/src/image/proxy.rs` | **Conservative narrow rewrite** from native Docker, Docker VM and rootless connectivity requirements. Shared pinned socat image/configuration and topology warrant treatment; standard socket/protocol terms may remain. Retain all three working routes and unrelated image logic. |
| Retired reconstruction research, inventories, briefs, prototype map and ZFS research | No at the listed original paths | **Drop from history** using [drop-paths.txt](uncloud-history/drop-paths.txt). Most are agent-authored research about upstream rather than copied Go; deletion is the map's cleanup policy, not a copied-source finding. |
| Renderer comparison comment and `excalidraw.example.uncld.dev` test hostname | Yes: renderer and renderer-success tests | **Rewrite in place**: describe the renderer's actual behavior; use a Ployz-owned synthetic test domain. Apply narrow historical wording replacement if removing branding from retained history. |
| Historical `opaque.uncloud.example` fixtures and reconstruction/TODO comments | Most have already changed in HEAD | **Rewrite in place** only at the named comments/fixture strings when sanitizing retained history; retain source implementation. Full path/version inventory is linked below. |
| `dns.uncloud.run` endpoint, protocol tests, verification instructions | Yes, CLI/daemon/tests | **Leave historical dependency facts**. Current use is removed/migrated by the Hosted DNS work; do not globally substitute a different endpoint in old commits, which would invent behavior those versions never had. |
| `ghcr.io/psviderski/unregistry:0.4.1`, preload commands, socket paths | Yes | **Leave**. These name the actual external image and platform integration. They do not establish that its implementation was copied into this repo. |
| Relay deployment references to Uncloud | Yes: `core/relay/{README.md,compose.yaml}`, `core/docs/RELEASE.md` | **Leave** while operationally accurate. Update current documentation when hosting changes; rewriting an accurate old deployment description is unnecessary. |
| `CONTEXT.md` “Avoid” vocabulary entries | Yes | **Leave**. A vocabulary exclusion is not copied implementation. |

The import's own [evidence manifest](https://github.com/getployz/ployz2/blob/1441cddec8bc1e04ee2570377691d9205e08e95a/evidence/README.md) says its fixtures and command pages are verbatim. Blob equality independently confirms all 71. The only additional exact object match is Git's empty blob, which is excluded as non-substantive.

## Source path and shape comparison

See [source comparison and dispositions](uncloud-history/source-comparison.md), [comparison manifest](uncloud-history/source-diffs-manifest.json), and [compressed exact unified diffs](uncloud-history/source-diffs.tar.xz). The manifest names local blob IDs, **all historical aliases**, upstream paths and pinned upstream blob IDs; `tar -xJf source-diffs.tar.xz` exposes the individual diffs.

The source investigation covers historical Go helpers, Rust counterparts to upstream Go module shapes, standard installer/platform integrations and store schema. It distinguishes retained dependency/API vocabulary from evidence requiring rewriting. Similarity thresholds select candidates; they do not prove the absence of translated or rearranged copying. Recommendations are per finding, not a claim that every implementation sharing a behavior should be deleted.

## Complete inventories

- [Every identifier-hit path, every matching blob, matching line numbers, HEAD status and action](uncloud-history/identifier-inventory.md).
- [Every exact upstream content match and every historical alias](uncloud-history/exact-matches.md).
- [Commit messages: 23 rewrites, 9 prune-if-empty candidates, 6 leave](uncloud-history/commit-messages.md). Full message bodies and merge commits were searched, not just subjects.
- [Explicit retired-artifact path filters](uncloud-history/drop-paths.txt). These contain no source-module glob.

The source findings and exact fixture matches add identifier-free content to the keyword inventory. An empty identifier search alone is not an acceptance criterion, and retained real dependency names mean an empty result is not desirable.

## Cutover handoff

1. Finish current-tree removals/rewrites above and the source-comparison dispositions. For copied fixtures, the planned Compose removal may satisfy the current-tree part; verify it rather than assuming it landed. Commit-message changes cannot substitute for this work.
2. In a disposable clone of the **selected export refs**, apply `git filter-repo` path removal for the retired artifacts. Remove the copied fixture blobs at every alias; if newly authored replacements share a path, filter by old blob IDs rather than deleting the whole path. Apply narrow replacements for the copied diagnostic and any approved branding cleanup. Keep an explicit leave-list for actual dependencies and platform facts.
3. Apply the message dispositions by original commit ID, preserving author information and co-author trailers. Let now-empty artifact-only commits prune. Retain the original-to-rewritten commit map outside the new public repository.
4. Repeat blob equality, identifier/path inspection, source-diff dispositions and message checks across **every ref to be exported**. Require all 71 known copied asset blobs to be unreachable, the copied diagnostic to be absent, all six identified historical schema blobs to be replaced, and each source rewrite disposition to be satisfied. Investigate new hits since this snapshot. Run the applicable product checks after current-tree changes.
5. Keep this audit asset and original evidence in the old repository for the cutover operator. The map already deletes audit documentation from the release tree; exclude this audit-only branch from the public export. Publish only the verified heads/tags. No force-push, repository rename, visibility change or content rewrite was performed by this audit.

## Reproduction

Use the same upstream baseline, and capture refs before scanning. The audit used these read-only history operations after fetching refs:

```sh
git rev-parse --is-shallow-repository
git for-each-ref --format='%(refname) %(objectname)'
git rev-list --all --count
git rev-list --objects --all
git log --all --root -m --no-renames --raw --no-abbrev --format='COMMIT %H'
git log --all --root -m --no-renames \
  -G '[Uu][Nn][Cc][Ll][Oo][Uu][Dd]|[Uu][Nn][Cc][Ll][Dd]|psviderski|ucind|ucnet' \
  --format='COMMIT %H' --name-only
git log --all --root -m --no-renames --pickaxe-regex \
  -S '[Uu][Nn][Cc][Ll][Oo][Uu][Dd]|[Uu][Nn][Cc][Ll][Dd]|psviderski|ucind|ucnet' \
  --format='COMMIT %H' --name-only
git log --all --format='%H%x00%B%x00'
git grep -n -i -E 'uncloud|uncld|psviderski|ucind|ucnet' HEAD
```

For completeness beyond pickaxe, `git cat-file --batch-check` identified every blob from `rev-list --objects --all`; `git cat-file --batch` read each blob by declared byte length and matched the case-insensitive byte expression. All old/new raw-diff object IDs supplied path aliases. Intersecting local and upstream blob-ID sets found identical contents regardless of filename or extension. Each substantive exact match was verified at the pinned upstream baseline. Source-specific comparisons and their limitations are documented with the diff archive. The combined manifest identifies 51 historical source blobs containing the copied DNS diagnostic, six public-IP policy candidates, twelve proxy-setup candidates and six Corrosion schemas; repeated comparisons of a blob are not counted as additional versions.
