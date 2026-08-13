# Frozen upstream evidence

All files below are verbatim inputs from [`psviderski/uncloud@b7e224a1eff98813b1d1a32034d977be24be994e`](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e). They are evidence, not source templates.

- `upstream/e2e-fixtures/` contains the 13 fixture files named by the reconstruction brief.
- `upstream/cli-reference/` contains the 58 generated Markdown command pages. The Docusaurus category file is excluded because it contains no command shape.
- `layer3.tsv` inventories every exact `t.Run` declaration and the 60 selected / 16 not-ported disposition fixed by [Decide Layer 3 upstream end-to-end coverage](https://github.com/getployz/ployz2/issues/18).
- `SHA256SUMS` freezes the copied bytes.

Run `scripts/check-evidence-inventories.sh` to verify counts, pinned sources, dispositions, the empty CLI deviation ledger, and copied-file checksums.
