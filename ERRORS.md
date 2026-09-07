# Error communication

How Ployz reports failure to its three consumers: the `ployz` CLI, the SDK
(`ployz-sdk`, a Node binding), and Ployz Cloud, which drives Ployz through the
SDK and renders its own UI. This governs the *words a person reads*, the *data a
program branches on*, and the *next step we hand them*. Mechanics (`thiserror`,
`?`, `expect`) live in [CODING_STANDARDS.md](CODING_STANDARDS.md#errors); this
is about content.

The surface is small and auditable. Every message is a `#[error("…")]` string
or a `Failure::usage(…)` call. In the CLI it is printed verbatim by
`terminate()` in `ployz/src/failure.rs`; across the wire it travels as
`RpcError { code, message, details }` and the SDK hands that object to Cloud as
JSON, unchanged. There is no second rendering layer anywhere. That is the
leverage — and the obligation: a bad `#[error]` string *is* a bad experience in
all three places, with nothing downstream to soften it.

## The shape of a good error

Three parts, in order. Two already-shipped errors are the reference:

**Storage** (`ployzd/src/volume_plugin/pool.rs`):

```
Not enough disk space on this machine to create {name}.
About {shortfall} more free space is needed, including storage overhead and OS reserve.
Free up disk space, expand the disk, or request a smaller volume.
```

**Post-join catch-up** (`ployz/src/global_catch_up.rs::joined_catch_up_error`):

```
Machine joined, but Global catch-up is incomplete; it remains a Cluster member. {cause}
Globals requiring attention:
- ployz-system/ingress: run `ployz ingress deploy`.
```

Both do the same three things:

1. **Situation** — what failed, naming the specific object. Not "a volume" —
   `{name}`. Not "a machine" — the Machine name/id. If the message could apply
   to two different objects, it is missing a name.
2. **State** — what is true now, especially whether the system is safe or
   partially changed. "it remains a Cluster member", "reset does not erase
   volume data on the host", "no mutations were replayed". A user's next move
   depends on whether the failed op left wreckage. Say so.
3. **Remediation** — the exact next step. A command in backticks
   (`` run `ployz ingress deploy` ``), a flag (`pass --yes to continue`), or a
   set of real options ("Free up disk space, expand the disk, or request a
   smaller volume"). "try again later" is not remediation; name the condition
   that has to change.

Not every error needs all three sentences — a pure input-validation error is
mostly part 1 + the correct form. But every error needs part 3 *or* to be
self-evidently actionable from part 1. "insufficient capacity on observed
eligible Machines" fails this: nothing tells the user whether to add a Machine,
wait for telemetry, or relax a constraint.

## One error, three consumers

```
leaf error (ployz-core / ployzd)
   │  Display = situation + state, consumer-neutral
   ▼
RpcError { code, message, details }        ← the wire; also the SDK's error object
   │                      │
   ▼                      ▼
ployz CLI                 SDK → Cloud
adds `ployz …` prose      branches on `code`, reads `details`,
(Failure wrapper)         renders its own button / next action
```

Each field has one job:

- **`message`** — parts 1–2 (situation, state) in plain words. Consumer-neutral:
  it **must not** name CLI commands, because Cloud shows the same string and
  `ployz deploy` is not a button. Remediation that is a real-world condition
  ("free disk space", "start Docker") is fine here; remediation that is a
  *verb* belongs to the consumer.
- **`code`** — the taxonomy kind (table below). This is what a program branches
  on. A user-input problem coded `Internal`, or a transient coded
  `InvalidArgument`, sends Cloud down the wrong path no matter how good the
  prose is.
- **`details`** — every object a consumer would need *as data* to offer a next
  step: the Machine ids, the Volume names, the hostname, the shortfall in
  bytes, the list of targets that failed. If it is interpolated into `message`
  and a consumer could act on it, it also goes in `details` as a frozen typed
  shape. Precedents: `ImageIngestReason` (`details.reason`,
  `ployz-core/src/rpc.rs`) and `UnconfirmedDataLoss` (whole struct,
  `ployz-core/src/domain/data_loss.rs`), each with a `from_details` /
  `from_rpc_error` reader so the consumer never parses prose.

The **CLI wrapper** adds part 3 (`ployz …`) the way `joined_catch_up_error`
wraps a bare `CatchUpError` and appends `run \`ployz ingress deploy\``. Cloud
adds its part 3 from `code` + `details`. Neither pushes its verbs into the leaf.

## Taxonomy

Every failure is one of these. The kind dictates parts 2–3.

| Kind | `RpcErrorCode` | User caused? | State to convey | Remediation (CLI prose / Cloud from `details`) |
|---|---|---|---|---|
| **Invalid input** | `InvalidArgument` | yes | nothing changed | the correct form (`use a positive integer followed by k, m, g, or t`) |
| **Not found** | `NotFound`, `Ambiguous` | usually | nothing changed | how to list valid names, or the candidates (`details`: the matches) |
| **Conflict** | `Conflict` | yes | nothing changed | which existing thing collides + how to pick another (`details`: the owner) |
| **Refused (needs confirmation)** | `InvalidArgument` + typed `details` | yes | nothing changed *yet* | the exact flag/repeat/confirmation to proceed; never auto-proceed on destructive loss (`UnconfirmedDataLoss` is the model) |
| **Unsupported / unauthenticated** | `Unsupported`, `Unauthenticated` | depends | nothing changed | what the target lacks, or how to re-enrol / re-pair |
| **Transient / unreachable** | `Unavailable` | no | whether the op is safe to rerun | the condition to fix (network, daemon down) + that rerun is safe |
| **Partial** | not an `RpcError`: a `PartialResult<T, E>` response with per-Machine `MachineFailure` rows | no | *which* targets succeeded/failed | the narrowest rerun that finishes the rest; each failed row carries its own coded error |
| **Internal (bug)** | `Internal` | no | the system may be inconsistent | report path (issue link / `ployz version` to include) |

Two rules fall out of the table:

- **Partial failures must enumerate.** `partial_failure_details` already lists
  `{machine}: {error}`. A partial that collapses to one line ("Global catch-up
  incomplete") without naming the machines/globals forces the user to guess the
  rerun scope.
- **Internal errors must be distinguishable from user errors.** An
  `RpcError::Internal` that prints a bare message ("boom") reads like the user
  did something wrong. `Failure` frames it once ("internal error: …" + the
  report step) and `RpcError` serialization adds `details.report` once, so no
  leaf message carries either.

## Destructive operations

Already the standard (PRs #763, #768); stated here so it stays:

- Name **every** object that will be destroyed, not a count.
- Irreversible data loss requires explicit per-name acceptance; `--yes` **cannot**
  bypass it. `--yes` only skips prompts for reversible or re-derivable changes.
- Non-terminal stdin without `--yes` is a *refusal*, not a silent default:
  "confirmation requires a terminal; pass --yes to continue".
- If the object set changed since the user confirmed, refuse and ask them to
  re-review ("Rerun to review the updated volume list") rather than acting on a
  stale confirmation.

The prompt itself has three separated parts — never one run-on sentence
(`handlers/data_loss.rs`, per #768). Consequences in plain language, the exact
reply on its own, the cancel path spelled out:

```
Removing app/db deletes its volumes and their data; this cannot be undone.
Type "app/db" to confirm:            ← the exact reply, alone
Press Enter without typing to cancel. ← the escape hatch, stated
```

"Type yes to confirm, or press Enter to cancel" for the non-per-name case.
Plain words, not internal jargon: "volume data is retained but loses cluster
access", not "observation" or "reset".

## Warnings vs. errors

A non-fatal follow-on (the command's main effect succeeded, a secondary step
did not) uses `Failure::warned` — one line, `WARNING: {context}: {cause}.`, and
it still fails the exit code. Don't bury a warning mid-output where it scrolls
away, and don't promote it to a hard error that hides the success.

## Formatting rules

- No Rust `Debug` of internal types in user text. `{0:?}` is fine for **echoing
  the user's own input** verbatim (a bad name, a bad `KEY=VALUE`) so they see
  exact bytes/quoting; it is wrong for internal structs, ids, or errors — those
  get a `Display`.
- No raw `Os { code: 111, … }` / `Status { … }` leaking through. `ConnectError`
  already peels these; new I/O and transport errors must too (there's a test:
  `exhausted_connections_print_how_many_were_tried`).
- One capitalization for domain nouns, matching `CONTEXT.md` (Machine, Cluster,
  Volume, Service, Global). Lowercase them and they read as different concepts.
- An interactive prompt states **both** how to proceed and how to cancel, each
  separated from the consequences — never fold "what happens", "type this", and
  "or back out" into one sentence (#768). A prompt that only tells the user how
  to say yes is a trap.
- End multi-sentence messages with periods; single-clause messages have none —
  match the two reference errors above.

## Enforcement

1. **Review checklist** (the real gate). For any touched `#[error]` or
   `Failure::usage`, the author states which taxonomy kind it is and, for
   non-input kinds, points at the remediation. `$four-axis-review` covers this
   under spec fidelity.
2. **A product path for the error seam.** A failure worth shipping is a failure
   worth a rung in `evidence/product-paths.tsv` asserting the *behavior* — the
   message names the object and offers the step — not the exact string
   (change-detector tests are rejected per CODING_STANDARDS). Model it on the
   existing `warned_follow_on_is_one_line_and_fails` and
   `post_join_ingress_error_names_membership_and_recovery` tests.
3. **Wire round-trip for anything a consumer acts on.** An error that crosses
   the RPC boundary with an actionable object gets a typed `details` shape and
   a `from_details` / `from_rpc_error` reader with a test, like
   `UnconfirmedDataLoss`. The reviewer asks: "could Cloud build the button from
   `code` + `details` alone, without reading `message`?" If not, it is missing
   data, not words.
4. **Cheap scan (optional, advisory).** A test can flag CLI-layer error strings
   under N words or matching a deny-list of terse dead-ends ("failed", "not
   running", "timed out") with no object name. Keep it advisory — it catches
   regressions, it does not prove an error is good.

## Known offenders (audit, 2026-09-07)

Full audit: 373 messages, 130 need rework, grouped into eleven families so each
is one PR. Internal framing at the seam (family 01) shipped with #772. Line numbers rot; the family and the file do not.

| family | files | fix shape |
|---|---|---|
| Can't reach anything | `context.rs`, `connect.rs`, `dns.rs` | name the Machine/socket; hand them `ployz machine init` / `ployz context use`; say rerun is safe |
| Debug-format sweep | `context.rs`, `operator.rs`, `machine/mod.rs`, core `deploy.rs`, `rpc.rs` | `{:?}` on paths/enums/ids/vecs → Display |
| Deploy capacity + planning | `deploy.rs`, core `deploy.rs`, `pipeline.rs`, `global_catch_up.rs` | name Machines/Service; add-a-Machine / wait / relax; scale refusal names mode + alternative |
| Operator / log selectors | `operator.rs` | name the Service; selector errors state the correct form |
| Provisioning + enroll | `provisioning.rs`, `cloud_enroll.rs` | say whether the Machine was touched and whether rerun is safe |
| Daemon Docker + machine | `ployzd/docker/mod.rs`, `machine/*.rs` | name the operation/container/peer; no `ployz` verbs (leaf) |
| Daemon certs + DNS | `certificates.rs`, `hosted_dns.rs`, `corrosion/certificate.rs` | hostname + plain-English status + condition to fix |
| Core Machine selectors | core `machine.rs`, `selector.rs` | name both Machines; list selectors comma-separated |
| CLI input odds and ends | `image.rs`, `compose/model.rs`, `volume.rs`, `project.rs`, `ingress/caddy.rs` | state the correct form or the alternative |
| Wire code mapping | `ployzd/machine_api/local.rs` (`hosted_dns_error`, `store_error`), `docker/mod.rs::rpc_code` | hosted-DNS input/status errors coded `Internal` → `InvalidArgument`/`Unavailable` |
