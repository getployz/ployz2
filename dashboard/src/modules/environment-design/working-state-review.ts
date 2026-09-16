import { canonicalWorkingReview, destructivePublication, destructivePublicationMismatch } from "@ployz/sdk/config";
import type { CompiledEnvironmentIntent } from "@ployz/sdk/config";
import { Schema } from "effect";
import { destructiveVolumeReviewsSchema } from "./destructive-volume-review";
import { environmentSavedStateBasisSchema } from "./saved-state";
import { Uuid } from "./workspace-schemas";

type ReviewedNodeSnapshot = {
  nodeType: "service" | "variable_group" | "volume";
  nodeId: string;
  nodeLineageId: string;
  configVersion: number;
  config: unknown;
};

export type ReviewableNodeIdentity = Pick<
  ReviewedNodeSnapshot,
  "nodeType" | "nodeId"
> & {
  config?: unknown | null;
};

export type DestructiveEnvironmentSave = {
  serviceIds: string[];
  volumeIds: string[];
};

export const workingStateFingerprintSchema = Schema.String.check(
  Schema.isPattern(/^environment-working-state-v1:[0-9a-f]{64}$/u),
);

export const destructiveServiceIdsSchema = Schema.mutable(
  Schema.Array(Uuid),
).check(
  Schema.makeFilter((serviceIds) =>
    new Set(serviceIds).size === serviceIds.length
      ? undefined
      : "A destructive Service can be reviewed only once.",
  ),
);

/**
 * Authority to publish one exact Working State revision as Saved State.
 *
 * The destructive set is required even when empty so every publisher is
 * checked against the same locked transition instead of opting into safety.
 */
export const environmentPublicationReviewSchema = Schema.Struct({
  savedStateBasis: environmentSavedStateBasisSchema,
  workingStateFingerprint: workingStateFingerprintSchema,
  destructiveServiceIds: destructiveServiceIdsSchema,
  destructiveVolumeReviews: destructiveVolumeReviewsSchema,
});

export type EnvironmentPublicationReview =
  typeof environmentPublicationReviewSchema.Type;

export function projectReviewedEnvironmentPublicationDestructiveSave(
  review: EnvironmentPublicationReview,
): DestructiveEnvironmentSave {
  return {
    serviceIds: review.destructiveServiceIds,
    volumeIds: review.destructiveVolumeReviews.map(
      (volumeReview) => volumeReview.target.resourceId,
    ),
  };
}

export const getDestructiveEnvironmentSaveReviewMismatch = destructivePublicationMismatch;
export const projectDestructiveEnvironmentSave = destructivePublication;

export type ReviewedEnvironmentWorkingState = {
  nodeSnapshots: ReviewedNodeSnapshot[];
  revisionMarkers?: string[];
};

/** The rendered document revision is the manual publication review boundary. */
export function projectReviewedEnvironmentWorkingState(document: {
  id: string; revision: string; compiled: CompiledEnvironmentIntent;
}): ReviewedEnvironmentWorkingState {
  return { nodeSnapshots: document.compiled.nodeSnapshots,
    revisionMarkers: [`environment:${document.id}:${document.revision}`] };
}

export const canonicalReviewedEnvironmentWorkingStateJson = canonicalWorkingReview;

export function formatReviewedEnvironmentWorkingStateFingerprint(
  digestHex: string,
): string {
  return `environment-working-state-v1:${digestHex}`;
}

export async function fingerprintReviewedEnvironmentWorkingState(
  input: ReviewedEnvironmentWorkingState,
): Promise<string> {
  const digest = await globalThis.crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(canonicalReviewedEnvironmentWorkingStateJson(input)),
  );
  return formatReviewedEnvironmentWorkingStateFingerprint(
    Array.from(new Uint8Array(digest), (byte) =>
      byte.toString(16).padStart(2, "0"),
    ).join(""),
  );
}
