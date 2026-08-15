# Frozen upstream evidence

All files below are verbatim inputs from [`psviderski/uncloud@b7e224a1eff98813b1d1a32034d977be24be994e`](https://github.com/psviderski/uncloud/blob/b7e224a1eff98813b1d1a32034d977be24be994e). They are evidence, not source templates.

- `upstream/e2e-fixtures/` contains the 13 fixture files named by the reconstruction brief.
- `upstream/cli-reference/` contains the 58 generated Markdown command pages. The Docusaurus category file is excluded because it contains no command shape.
- `layer1.tsv` maps the exact 86 top-level semantic cases in the approved Layer 1 families to Rust tests or explicit Rust-structure non-ports. Each row accounts for the complete assertion and subcase body of its upstream case. The count is mechanically derived from the pinned source inventory: 33 Compose, 21 Deploy, 12 API/domain, 13 client Caddy/DNS/log-merging, four `cmd/uc` and config/connection cases, and three binding Caddyfile cases. The earlier `~85` estimate predates this exact enumeration; the dropped Caddy JSON generator remains excluded by [#24](https://github.com/getployz/ployz2/issues/24).
- `layer3.tsv` inventories every exact `t.Run` declaration and the 60 selected / 16 not-ported disposition fixed by [Decide Layer 3 upstream end-to-end coverage](https://github.com/getployz/ployz2/issues/18).
- `SHA256SUMS` freezes the copied bytes.

Run `scripts/check-evidence-inventories.sh` to verify counts, pinned sources, dispositions, the CLI deviation-ledger format, and copied-file checksums.
