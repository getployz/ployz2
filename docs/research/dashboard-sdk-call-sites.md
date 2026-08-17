# What Ployz Cloud calls on `@ployz/sdk` today

Ticket: [What does Ployz Cloud call on @ployz/sdk today](https://github.com/getployz/ployz2/issues/301).
Parent map: [Give Cloud a first-party SDK over the relay](https://github.com/getployz/ployz2/issues/299).

## Answer

Ployz Cloud (`ployz-cloud` in `getployz/ployz-dashboard`) is a NATS operator of published `@ployz/sdk@0.0.2-alpha.89`. It connects with `connectPloyzNatsClient` using Cloud Connection `tls://host:4222` + CA + decrypted nkey seed. It reads cluster truth through `watchRuntime` / `runtime.snapshot` / `volume.list` / `build.target.capabilities`. It issues commands through `transport.request` (not the SDK’s single-service `PloyzClient.deploy()`): `deploy.preview`, `deploy.reserve` + `deploy.submit`, `build.submit` / `build.cancel`, `machine.storage_prepare`, `volume.remove`, `credential.add` / `credential.remove`. Cache prune is the one high-level `PloyzClient` command: `machineBuildCachePrune` then `OperationHandle.status()`. Progress for deploy/build/storage-prep/volume-remove is paged `ops.watch` (limit 100); credentials and prune poll `ops.status`. Joiner bootstrap issues `machine.add` as a **raw NATS request** using `@ployz/sdk/generated` types, not `PloyzClient.machineAdd()`.

## Sources

| What | Where | Ref |
| --- | --- | --- |
| Cloud source | [getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) default branch | `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde` |
| Pin | dashboard `package.json` dependency `"@ployz/sdk": "0.0.2-alpha.89"` | same SHA |
| Published package | npm `@ployz/sdk@0.0.2-alpha.89` (`git+https://github.com/getployz/ployz.git`) | tarball `sdk-0.0.2-alpha.89.tgz` |
| Types Cloud compiles against | [getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk` at tag `v0.0.2-alpha.89` | `d173690c39d9a8f02c8be190810328a6d6fe185f` |

Do not use `getployz/ployz` default-branch `packages/ployz-sdk` for this inventory. Tag `v0.0.2-alpha.89` ships `connectPloyzNatsClient` / `nats.ts`; current default-branch `index.ts` does not. The git `packages/ployz-sdk/package.json` at that tag still says `"version": "0.0.2-alpha.28"`; npm rewrites it to `0.0.2-alpha.89` on publish ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/package.json` @ `v0.0.2-alpha.89`; npm tarball `package.json`).

GitHub code search on the private dashboard returned zero indexed hits. This inventory is a directory walk of production trees (`src/models/{runtime,services,builds,servers,operations}`, `src/db`, `src/routes/api/{runtime,bootstrap}`, `src/inggest`, `src/lib`, `src/utils`) plus every file those trees imported `@ployz/sdk` or `@ployz/sdk/generated` from. UI collections consume Cloud lenses, not the SDK.

Citations below are `repo` + `path` + `ref`.

## How Cloud connects

`connectPloyzNatsClient({ nats }` or `{ nats, requestTimeoutMs })` → `ConnectedPloyzClient` `{ client: PloyzClient, transport: PloyzNatsTransport, close(), drain() }` ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/index.ts` @ `v0.0.2-alpha.89`).

NATS options come from a Cloud Connection row: `runtimeNatsUrl`, `trustedNats.ca_pem`, decrypted `encryptedCloudNatsSeed`, `inboxPrefix: "_INBOX_operator"`, TLS, nkey authenticator ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/servers/cloud-bootstrap-bridge.server.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`; `src/models/runtime/runtime-client.server.ts` same SHA).

Shared wrapper: `connectRuntimeClient` / `openRuntimeClient` / `openRuntimeClientForOrganization` / `openRuntimeWatch` in `src/models/runtime/runtime-client.server.ts` (same SHA). `openRuntimeWatch` calls `connected.client.watchRuntime()` and returns `{ snapshots, close }` or `{ status: "no_connection" }`.

Transport: JSON over NATS subjects from `OPERATION_API_CONTRACTS`; default request timeout 10s ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/nats.ts` @ `v0.0.2-alpha.89`). Cloud often uses `transport.request` directly instead of `PloyzClient` helpers.

```text
Cloud  --connectPloyzNatsClient-->  Core NATS (tls://host:4222)
  |                                    |
  |  watchRuntime / runtime.snapshot   |  lens
  |  volume.list / build.target.capabilities
  |
  |  transport.request(command)        |  commands
  |  ops.watch / ops.status            |  observation
```

## Lens reads

### `watchRuntime` → SSE `runtime.lens`

| | |
| --- | --- |
| Module | `src/models/runtime/runtime-client.server.ts` |
| Method | `connected.client.watchRuntime()` |
| Payload | none. Transport seeds `plz.v1.rpc.operator.query.runtime.snapshot.seed` with `{}` and subscribes to `plz.v1.projection.runtime.snapshot` ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/generated.ts` `RUNTIME_SNAPSHOT_SEED` / `RUNTIME_SNAPSHOT_STREAM`; `packages/ployz-sdk/src/nats.ts` @ `v0.0.2-alpha.89`) |
| Result | `AsyncIterable<RuntimeSnapshot>` |

`RuntimeSnapshot` fields Cloud projects: `automatic_hostname_configuration`, `ployz_dns_target`, `machines`, `services`, `route_tls`, `updated_at_unix_seconds` ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/generated.ts` type `RuntimeSnapshot` @ `v0.0.2-alpha.89`; projector [getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/runtime/runtime-snapshot.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`).

Cloud maps each snapshot through `runtimeSnapshotLensFromSnapshot` and SSE-emits `event: runtime.lens` ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/runtime/runtime-events.server.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`). Route: `src/routes/api/runtime/events.handler.ts` calls `openRuntimeWatch` then `createRuntimeEventsResponse` (same SHA). Browser collections (`src/models/runtime/runtime.collection.ts`, `use-runtime-lens.ts`) consume that Cloud lens, not `@ployz/sdk`.

### `runtime.snapshot` (one-shot)

| | |
| --- | --- |
| Module | `src/models/runtime/runtime-snapshot-query.server.ts` |
| Method | `connected.client.runtimeSnapshot()` → `transport.request("runtime.snapshot", {})` |
| Payload | empty (`RuntimeSnapshotRequest = Record<symbol, never>`) |
| Result | SDK unwraps `RuntimeSnapshotResult.snapshot`. Cloud projects the same lens. On error/invalid snapshot: `{ status: "unavailable" }` lens, not a thrown domain error. No connection: `{ status: "no_connection" }` ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/runtime/runtime-snapshot-query.server.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`) |

Second caller: storage-prep admission reads a fresh roster via `client.runtimeSnapshot()` with a 5s timeout, then classifies the machine as eligible/rejected ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/servers/cloud-storage-preparation-runtime-adapter.server.ts`; `src/models/servers/cloud-storage-preparation-admission.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`).

### `volume.list`

| | |
| --- | --- |
| Module | `src/models/runtime/runtime-volume-query.server.ts` |
| Method | `transport.request("volume.list", {})` |
| Timeout | 5s (`RUNTIME_VOLUME_REQUEST_TIMEOUT_MS`) |
| Payload | empty (`VolumeListRequest`) |
| Result | `{ status: "current", volumes: VolumeSnapshot[] }` or `{ status: "unavailable", reason: "no_connection" \| "domain_error" }` |

`VolumeSnapshot`: `{ namespace_id, volume_name, machine_id, kind, referencing_services, testimony, status }` ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/generated.ts` @ `v0.0.2-alpha.89`). Cloud re-validates with Zod before trusting the list ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/runtime/runtime-volume-query.server.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`). Downstream: provisioned-storage lens (`src/models/runtime/provisioned-storage-lens.ts`, type-only `VolumeSnapshot`); storage history page (`src/models/runtime/provisioned-storage-read.server.ts`).

Same RPC for destructive-volume confirmation: `verifyFreshDestructiveVolumeEvidence` lists volumes and compares reviewed testimony ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/operations/destructive-volume-runtime-adapter.server.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`).

### `build.target.capabilities`

| | |
| --- | --- |
| Module | `src/models/builds/build-target-readiness.server.ts` |
| Method | `transport.request("build.target.capabilities", {})` |
| Timeout | 5s |
| Payload | empty |
| Result | `{ status: "pending" }` (no connection), `{ status: "current", capabilities }` (Zod-parsed cluster machines + external pools), or `{ status: "unavailable" }` |

Type on the pin: `{ cluster: { machines: [...] }, external_pools: [...] }` ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/generated.ts` `BuildTargetCapabilities` @ `v0.0.2-alpha.89`). Cloud does not keep the SDK type; it re-parses with Zod ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/builds/build-target-readiness.server.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`).

## Commands

Cloud almost always talks `transport.request(endpoint, request)` and inspects `{ status: "ok" | "domain_error" }`. It does **not** call `PloyzClient.deploy()` (that helper builds a single-service `PloyzDeployInput`; Cloud submits a full namespace `DeployRequest` with volumes, phases stripped, and registry credentials) ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/index.ts` `PloyzClient.deploy` / `PloyzDeployInput` @ `v0.0.2-alpha.89`; [getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/services/deploy-runtime-adapter.server.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`).

Phase-aware Cloud requests currently send `request.target` only (`toCurrentRuntimeDeployTarget`); Runtime does not yet take phases (`TODO(#894)`) ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/runtime/phase-aware-deploy-current-runtime-adapter.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`).

### `deploy.preview`

| | |
| --- | --- |
| Module | `src/models/services/deploy-runtime-adapter.server.ts` `previewDeploy` |
| Method | `transport.request("deploy.preview", request)` |
| Payload | `DeployPreviewRequest`: `{ target: DeployPreviewTarget, registry_credentials? }`. Target is a namespace with volumes + services; git services may be `{ state: "pending_build" }` images ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/generated.ts` @ `v0.0.2-alpha.89`; compiler [getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/services/environment-deployments.ts` `compileEnvironmentDeployPreviewRequest` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`) |
| Result | `DeployPreview` (projection, build platform requirements, unusable machines) or `DeploymentExecutionError` with `core_preview_*`. Capacity errors get a GiB-formatted operator message |

### `deploy.reserve` + `deploy.submit`

| | |
| --- | --- |
| Module | `src/models/services/deploy-runtime-adapter.server.ts` `submitFrozenDeployOperation` |
| Methods | `transport.request("deploy.reserve", { namespace_id })` then `transport.request("deploy.submit", request)` |
| Payload | Reserve: `{ namespace_id }` from frozen target. Submit: `{ idempotency_key: operationIdempotencyKey(environmentDeploymentId), reservation_id, target: toCurrentRuntimeDeployTarget(frozen.request), registry_credentials }` — `DeploySubmitRequest` ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/generated.ts` @ `v0.0.2-alpha.89`) |
| Result | `{ operationId, proposedStartSequence }`. On `reservation_already_committed`, reuses `owner_operation_id` with start sequence `"1"` |

Frozen input is encrypted JSON (v1–v3) restored to branded SDK types (`DeployRequest`, `RegistryCredential`, mount/route brands) ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/services/frozen-deploy-input.server.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`). Inngest `src/inggest/functions/environment-deployments/deploy.ts` orchestrates submit/watch/close via those adapters; it does not import `@ployz/sdk` itself.

### `build.submit` / `build.cancel`

| | |
| --- | --- |
| Module | `src/models/builds/build-runtime-adapter.server.ts` |
| Methods | `transport.request("build.submit", request)`; `transport.request("build.cancel", { operation_id, reason: cancellationReason(...) })` |
| Payload | `BuildSubmitRequest`: `{ operation_id, target?: { target: "cluster" } \| { target: "external", pool_id }, source: git\|local_snapshot, adapter: dockerfile\|railpack, platforms }` ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/generated.ts` @ `v0.0.2-alpha.89`). Git source credential is minted as `{ username: "x-access-token", secret: token }` ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/builds/build-attempt-executor.server.ts` `submissionRequestFor` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`) |
| Result | Accepted `{ operationId, proposedStartSequence }` or declined (`no_capable_external_executor`, `no_reachable_image_seed`, local-snapshot policy errors, `unavailable`). `operation_conflict` throws non-retriable. Cancel: `cancelled` / `already_terminal` / `no_such_operation` |

GitHub Actions target executor also calls `submitBuildOperation` after dispatch ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/builds/github-actions-target-executor.server.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`).

### `machine.storage_prepare`

| | |
| --- | --- |
| Module | `src/models/servers/cloud-storage-runtime-adapter.server.ts` |
| Method | `transport.request("machine.storage_prepare", request)` |
| Payload | `{ operation_id: storagePreparationOperationId(attemptId), machine_id, pool }` (`MachineStoragePrepareRequest`; pool optional) |
| Result | `AcceptedOperation`; rejects if Core returns a different `operation_id` |

### `volume.remove`

| | |
| --- | --- |
| Module | `src/models/operations/destructive-volume-runtime-adapter.server.ts` |
| Method | `transport.request("volume.remove", { operation_id: operationId(attemptId), namespace_id, volume_name })` |
| Result | Accepted `{ operation_id, start_sequence }`. `resource_busy` is success if `owner_operation_id` is this attempt. Recovery uses `ops.status` expecting `status.kind === "volume_remove"` |

### `machine.build_cache_prune`

| | |
| --- | --- |
| Module | `src/models/servers/machine-build-cache-prune.server.ts` |
| Method | `connected.client.machineBuildCachePrune({ operationId, machineId })` then `status()` in a 120s / 250ms poll |
| Payload | `{ operation_id, machine_id }` (`op_cache_prune_${uuid}`) |
| Result | On `state === "completed"`, returns `BuildCachePruneEvidence` `{ before_available_bytes, reclaimed_bytes, after_available_bytes }`. Expects `status.kind === "machine_build_cache_prune"` |

This is the only production use of a `PloyzClient` command helper besides `runtimeSnapshot` / `watchRuntime` ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/index.ts` `machineBuildCachePrune` @ `v0.0.2-alpha.89`). Server fn brands the machine id: `src/models/servers/machine-build-cache-prune.functions.ts`.

### `credential.add` / `credential.remove`

| | |
| --- | --- |
| Module | `src/models/builds/build-executor-authority-core.server.ts` |
| Methods | `transport.request("credential.add", { operation_id, grant })`; `transport.request("credential.remove", { operation_id, public_key })` |
| Timeout | connect 5s; poll `ops.status` until completed (30s, 250ms). Does **not** use `ops.watch` |
| Payload | Grant: `{ public_key, name, role: { build_executor: { pool_id, executor_id, expires_at } } }` (`CredentialGrant`) ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/generated.ts` @ `v0.0.2-alpha.89`) |
| Result | Idempotent: if `ops.status` already shows the same action, skip submit. Homelab pool enroll/renew/revoke drives this ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/builds/homelab-build-pool.server.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`) |

## Operation observation

### `ops.watch`

| | |
| --- | --- |
| Module | `src/models/operations/core-operation-evidence.server.ts` |
| Method | `transport.request("ops.watch", { operation_id, start_sequence, limit })` |
| Payload | `OpsWatchRequest` = `{ operation_id, start_sequence: EventSequence, limit: OperationEventReplayLimit }`. Cloud uses `limit: 100` (`REPLAY_PAGE_SIZE`) ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/generated.ts` `OperationEventReplayRequest` @ `v0.0.2-alpha.89`) |
| Result | `OperationEventReplayPage` `{ events: ReplayedOperationEvent[], cursor: more\|caught_up\|terminal }`. Cloud validates contiguous sequences, projects by expected kind, persists `coreOperationEvent` rows, advances `coreOperationWatch` |

Callers: deploy (`watchDeployOperationBatch` + `projectDeployOperationEvent`), build (`observeBuildWithTransport` + `projectBuildOperationEvent`), storage-prep (`watchMachineStoragePreparationBatch` + `projectMachineStoragePrepareOperationEvent`), volume-remove (`watchDestructiveVolumeBatch` + `projectDestructiveVolumeOperationEvent`). UI reads persisted pages (`use-deployment-operation-evidence.ts`), not the SDK.

### `ops.status`

Used as a poll, not a live watch:

- Builds: expect `status.kind === "build"` (`readBuildWithTransport`).
- Credentials: expect `status.kind === "credential_grant"` matching add/remove action.
- Volume-remove recovery: expect `status.kind === "volume_remove"` for the reviewed namespace/name.
- Cache prune: via `OperationHandle.status()` expecting `machine_build_cache_prune`.

Payload: `{ operation_id }` → `{ status: OperationStatus }` ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/generated.ts` @ `v0.0.2-alpha.89`).

## `machine.add`: generated types, raw NATS, not `PloyzClient`

Joiner bootstrap **does** call the `machine.add` RPC. It does **not** go through `@ployz/sdk`’s client/transport.

`requestCloudMachineAdd` connects with `@nats-io/transport-node` and `connection.request("plz.v1.rpc.operator.command.machine.add", JSON.stringify(request), { timeout: 10_000 })`, then `responseMessage.json<MachineAddResponse>()` ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/servers/cloud-bootstrap-bridge.server.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`).

Payload (`MachineAddRequest`): `{ operation_id: op_cloud_add_<redemption>, idempotency_key: idem_cloud_add_<redemption>, machine_id: machine_<redemption>, name, roles: { gateway: "install" }, host_port_assurance: "keeper" }` (`buildCloudMachineAddRequest`, same file). Types imported from `@ployz/sdk/generated`.

Result: `MachineAddAccepted` (`join_token`, `join_secret_delivery`, …) becomes the joiner envelope ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/servers/cloud-bootstrap-redemptions.server.ts` `joinerIntent` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`). Founder path does not call `machine.add`; it issues founder NATS material and later proves reachability with a raw `connect` + `flush` (`proveCloudNatsReachability`).

A contract test freezes the `machine.add` **subject** even though production never uses `PloyzClient.machineAdd()` ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/runtime/ployz-sdk-contract.test.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`).

Cloud bootstrap HTTP also uses `@ployz/sdk/generated` for envelope/callback/decision types (`CloudBootstrapEnvelope`, `CloudBootstrapCallbackRequest`, `CloudBootstrapDecision`, machine facts) — host-runner protocol, not operator RPC ([getployz/ployz-dashboard](https://github.com/getployz/ployz-dashboard) `src/models/servers/cloud-bootstrap-request.ts`, `cloud-bootstrap-tokens.server.ts`, `cloud-bootstrap-callbacks.server.ts`, `cloud-bootstrap-terminal.server.ts`, `cloud-bootstrap-redemption-policy.ts`, `cloud-bootstrap-redemption-source.ts` @ `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`; types [getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/generated.ts` @ `v0.0.2-alpha.89`).

## Type-only / payload construction (no live RPC in that file)

| Module | SDK import | Role |
| --- | --- | --- |
| `src/models/services/environment-deployments.ts` | `imageDefaultRuntime`, brands, `DeployRequest` / `DeployPreviewRequest` | Compile Cloud service config → Core deploy/preview target |
| `src/models/services/environment-deployments.server.ts` | `serviceId`, `PushedImageReceipt`, `RegistryCredential` | Orchestrate preview/submit; no direct RPC |
| `src/models/services/frozen-deploy-input.server.ts` | `DeployRequest`, brands | Encode/decode encrypted frozen deploy |
| `src/models/environment-resources/volume-config.ts` | `VolumeSpec`, `VolumeMaxSizeBytes` | Map Cloud volume storage → deploy spec |
| `src/models/runtime/runtime-snapshot.ts` | `RuntimeSnapshot` | Project SDK snapshot → Cloud lens |
| `src/models/runtime/provisioned-storage-lens.ts` | `VolumeSnapshot` | Combine volume list + machine storage |
| `src/models/runtime/runtime-snapshot.test-fixture.ts` | `RuntimeSnapshot` | Test fixture |
| `src/models/operations/core-operation-evidence.server.ts` | `operationId`, `eventSequence`, `operationEventReplayLimit`, `OperationEvent`, `PloyzOperationTransport` | Shared `ops.watch` pager |
| `src/models/operations/deploy-operation-evidence.ts` | `OperationEvent`, deploy failure types | Compact deploy events for Electric |
| `src/models/operations/build-operation-evidence.ts` | `OperationEvent` | Compact build events |
| `src/models/operations/destructive-volume-operation-evidence.ts` | `OperationEvent` | Compact volume-remove events |
| `src/models/operations/destructive-volume-evidence.ts` | `@ployz/sdk/generated` `VolumeSnapshot`, `NamespaceId` | Testimony fingerprint |
| `src/models/operations/destructive-volume-attempt.ts` | `OperationId` | Attempt state machine |
| `src/models/servers/cloud-storage-preparation.ts` | `@ployz/sdk/generated` operation/event types; `machineId`/`operationId` from `@ployz/sdk` | Storage-prep domain |
| `src/models/builds/build-target-policy.ts` | `BuildTarget` | Frozen target policy |
| `src/models/builds/build-attempt-executor.server.ts` | `BuildSubmitRequest`, `operationId` | Build `BuildSubmitRequest` then call adapter |
| `src/db/schema.ts` | `@ployz/sdk` + `@ployz/sdk/generated` | Persist SDK-shaped JSON (previews, receipts, bootstrap envelopes, operation events, storage-prep state) |
| `src/models/runtime/ployz-sdk-contract.test.ts` | `OPERATION_API_CONTRACTS` | Freeze five subjects |

## SDK surface Cloud does not call

Present on the pin’s `OPERATION_API_CONTRACTS` / `PloyzClient` ([getployz/ployz](https://github.com/getployz/ployz) `packages/ployz-sdk/src/generated.ts` and `src/index.ts` @ `v0.0.2-alpha.89`). Not observed as a Cloud SDK/`transport.request` call:

- `PloyzClient.deploy()` (single-service helper)
- `PloyzClient.machineAdd()` / `initFirstMachineActivate` / `machineJoinRedeem`
- `system.deploy`, `machine.update`, `machine.drain` / `machine.resume`
- `volume.create`, `namespace.remove`, `service.restart`
- `core.replace` / `core.replace.report`
- `credential.list`
- `ingress.configure`
- `machine.list` / `machine.inspect` (roster comes from `runtime.snapshot` / `watchRuntime`)
- `service.list` / `service.inspect`
- `network.status` / `network.resolve` / `network.repair`
- `machine.redeem` / `machine.report` (host-runner join path, not Cloud’s operator client)
- `logs.tail`, `ops.list`

`machine.add` is issued, but only as raw NATS in bootstrap (above).

## Import inventory (production)

Every production file found importing `@ployz/sdk` or `@ployz/sdk/generated` on dashboard `bf8ca4c6e5b6ef12fc11ab7bbee1c6d924164bde`:

**Connect / RPC**

- `src/models/runtime/runtime-client.server.ts` — `connectPloyzNatsClient`, `RuntimeSnapshot`; `watchRuntime`
- `src/models/services/deploy-runtime-adapter.server.ts` — connect + `deploy.preview` / `reserve` / `submit`
- `src/models/builds/build-runtime-adapter.server.ts` — connect + `build.submit` / `cancel` / `ops.status` / `ops.watch`
- `src/models/builds/build-executor-authority-core.server.ts` — connect + `credential.add` / `remove` / `ops.status`
- `src/models/servers/cloud-storage-runtime-adapter.server.ts` — connect + `machine.storage_prepare` / `ops.watch`
- `src/models/operations/destructive-volume-runtime-adapter.server.ts` — connect + `volume.list` / `volume.remove` / `ops.status` / `ops.watch`
- `src/models/servers/machine-build-cache-prune.server.ts` — `machineBuildCachePrune` + `status`
- `src/models/runtime/runtime-snapshot-query.server.ts` — `runtimeSnapshot` (via wrapper)
- `src/models/runtime/runtime-volume-query.server.ts` — `volume.list`
- `src/models/builds/build-target-readiness.server.ts` — `build.target.capabilities`
- `src/models/servers/cloud-storage-preparation-runtime-adapter.server.ts` — `runtimeSnapshot`
- `src/models/operations/core-operation-evidence.server.ts` — `ops.watch`
- `src/models/servers/cloud-bootstrap-bridge.server.ts` — `@ployz/sdk/generated` types; raw NATS `machine.add`

**Types / brands / compilers**

- `src/models/runtime/runtime-events.server.ts`, `runtime-snapshot.ts`, `provisioned-storage-lens.ts`
- `src/models/services/environment-deployments.ts`, `environment-deployments.server.ts`, `frozen-deploy-input.server.ts`
- `src/models/environment-resources/volume-config.ts`
- `src/models/operations/build-operation-evidence.ts`, `deploy-operation-evidence.ts`, `destructive-volume-operation-evidence.ts`, `destructive-volume-evidence.ts`, `destructive-volume-attempt.ts`
- `src/models/servers/cloud-storage-preparation.ts`, `cloud-storage-preparation-admission.ts`
- `src/models/servers/cloud-bootstrap-request.ts`, `cloud-bootstrap-redemption-source.ts`, `cloud-bootstrap-redemption-policy.ts`, `cloud-bootstrap-redemptions.server.ts`, `cloud-bootstrap-tokens.server.ts`, `cloud-bootstrap-callbacks.server.ts`, `cloud-bootstrap-terminal.server.ts`
- `src/models/builds/build-target-policy.ts`, `build-attempt-executor.server.ts`, `github-actions-target-executor.server.ts`
- `src/models/servers/machine-build-cache-prune.functions.ts`
- `src/db/schema.ts`

**Contract test**

- `src/models/runtime/ployz-sdk-contract.test.ts`
- `src/models/runtime/runtime-snapshot.test-fixture.ts`
