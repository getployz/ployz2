import type { DeployIntent } from "@ployz/sdk";
import { lowerDeployment } from "@ployz/sdk/config";
import { Schema } from "effect";
import type { JsonValue } from "#/db/tables";
import type { EnvironmentDeploymentPreview } from "#/modules/deployments/tables";
import { projectJsonValue } from "#/lib/json";
import { strictParseOptions } from "#/modules/environment-design/schema";
import {
  type EnvironmentDeploySnapshot,
  type EnvironmentDeployVolume,
} from "#/modules/deployments/runtime-contract";

export const runtimeDeployPreviewSchema = Schema.Struct({
  storage: Schema.optional(Schema.mutable(Schema.Array(Schema.Json))),
  project_name: Schema.String.check(Schema.isNonEmpty()),
  prune_refusal: Schema.optionalKey(Schema.NullOr(Schema.Literals([
    "incomplete_snapshot", "selected_services", "filtered_profiles", "guessed_project_name",
  ]))),
  operations: Schema.mutable(Schema.Array(Schema.Json)),
  warnings: Schema.mutable(Schema.Array(Schema.Json)),
  would_remove: Schema.mutable(Schema.Array(Schema.Json)),
  volumes_to_create: Schema.optional(
    Schema.mutable(Schema.Array(Schema.Json)),
  ),
  preserved_volumes: Schema.mutable(Schema.Array(Schema.Json)),
});

export const runtimeDeployOutcomeSchema = Schema.Union([
  Schema.Struct({ type: Schema.Literal("success"), completed: Schema.Array(Schema.Unknown) }),
  Schema.Struct({
    type: Schema.Literal("failed"),
    completed: Schema.Array(Schema.Unknown),
    failed: Schema.Struct({ error: Schema.Struct({ type: Schema.Literals([
      "machine", "health", "dependency_health", "hook", "cancelled",
    ]) }) }),
    unexecuted: Schema.Array(Schema.Unknown),
  }),
]);

export type SdkDeployPreview = EnvironmentDeploymentPreview;

function mutableJsonArray(values: readonly Schema.Json[]): JsonValue[] | null {
  const projected = values.map(projectJsonValue);
  if (projected.some((value) => value === undefined)) {
    return null;
  }
  // SAFETY: the guard above proves every projected item is a JsonValue.
  return projected as JsonValue[];
}

export function projectRuntimeDeployPreview(
  decoded: typeof runtimeDeployPreviewSchema.Type,
): SdkDeployPreview | null {
  const storage = decoded.storage === undefined
    ? undefined
    : mutableJsonArray(decoded.storage);
  const operations = mutableJsonArray(decoded.operations);
  const warnings = mutableJsonArray(decoded.warnings);
  const wouldRemove = mutableJsonArray(decoded.would_remove);
  const volumesToCreate =
    decoded.volumes_to_create === undefined
      ? undefined
      : mutableJsonArray(decoded.volumes_to_create);
  const preservedVolumes = mutableJsonArray(decoded.preserved_volumes);
  if (
    storage === null ||
    operations === null ||
    warnings === null ||
    wouldRemove === null ||
    volumesToCreate === null ||
    preservedVolumes === null
  ) {
    return null;
  }
  const preview: SdkDeployPreview = {
    project_name: decoded.project_name,
    operations,
    warnings,
    would_remove: wouldRemove,
    preserved_volumes: preservedVolumes,
  };
  if (storage !== undefined) {
    preview.storage = storage;
  }
  if (decoded.prune_refusal !== undefined) {
    preview.prune_refusal = decoded.prune_refusal;
  }
  if (volumesToCreate !== undefined) {
    preview.volumes_to_create = volumesToCreate;
  }
  return preview;
}

export function compileSdkDeployIntent(input: {
  projectName: string;
  snapshots: readonly EnvironmentDeploySnapshot[];
  volumes?: readonly EnvironmentDeployVolume[];
}): DeployIntent {
  return lowerDeployment(input);
}

export function parseSdkDeployPreview<T>(value: T): SdkDeployPreview {
  const preview = projectRuntimeDeployPreview(
    Schema.decodeUnknownSync(runtimeDeployPreviewSchema)(
      value,
      strictParseOptions,
    ),
  );
  if (preview === null) {
    throw new Error("SDK deploy preview contains non-JSON data.");
  }
  return preview;
}
