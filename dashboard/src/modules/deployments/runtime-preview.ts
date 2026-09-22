import { toCoreServiceConfig } from "#/modules/environment-design/service-config";
import type { DeployIntent } from "@ployz/sdk";
import { lowerDeployment, parseRuntimePreview } from "@ployz/sdk/config";
import { Schema } from "effect";
import type { EnvironmentDeploymentPreview } from "#/modules/deployments/tables";
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

export type SdkDeployPreview = EnvironmentDeploymentPreview;

export function compileSdkPreparationInput(input: {
  projectName: string;
  snapshots: readonly EnvironmentDeploySnapshot[];
  volumes?: readonly EnvironmentDeployVolume[];
}): Parameters<typeof lowerDeployment>[0] {
  return {
    projectName: input.projectName,
    snapshots: input.snapshots.map((snapshot) => ({
      serviceId: snapshot.serviceId,
      config: toCoreServiceConfig(snapshot.config),
      replicas: snapshot.replicas,
      resolvedEnv: snapshot.resolvedEnv,
    } satisfies Parameters<typeof lowerDeployment>[0]["snapshots"][number])),
    volumes: input.volumes,
  };
}

export function parseSdkDeployPreview<T>(value: T): SdkDeployPreview {
  return parseRuntimePreview(value);
}

export function compileSdkDeployIntent(input: Parameters<typeof compileSdkPreparationInput>[0]): DeployIntent {
  return lowerDeployment(compileSdkPreparationInput(input));
}
