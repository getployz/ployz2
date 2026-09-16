import "@tanstack/react-start/server-only";

import { Effect } from "effect";
import { admitEnvironmentDeployment } from "#/modules/deployments/admission.server";
import type {
  EnvironmentPublicationSubmissionOutcome,
  ReviewedPublicationInput,
} from "#/modules/deployments/deployment-contract";
import { gatherExactTombstonedVolumeReviews } from "#/modules/deployments/destructive-volume-review.server";
import { dispatchEnvironmentDeployment } from "#/modules/deployments/dispatch.server";
import { loadEnvironmentSnapshotProjection } from "#/modules/deployments/environment-state.repository.server";
import { findUnpullableSdkDeployImages } from "#/modules/deployments/image-gate";
import { isActiveDeploymentUniqueViolation } from "#/modules/deployments/queue-lock.server";
import { requireEnvironmentForActor } from "#/modules/environment-design/authoring-repository.server";
import {
  DestructiveVolumeReviewChangedError,
  getDestructiveVolumeReviewMismatch,
} from "#/modules/environment-design/destructive-volume-review";
import { saveReviewedEnvironmentState } from "#/modules/environment-design/saved-state-operations.server";
import {
  loadCurrentEnvironmentSnapshotProjection,
  loadEnvironmentDocument,
} from "#/modules/environment-design/working-state-repository.server";
import {
  projectDestructiveEnvironmentSave,
  type EnvironmentPublicationReview,
} from "#/modules/environment-design/working-state-review";
import type { Actor } from "#/modules/identity/actor";
import { Database } from "#/server/database.server";
import { withMutationResult } from "#/server/mutation-result.server";
import { Validation } from "#/server/public-error";

type EnvironmentContextInput = {
  readonly organizationSlug: string;
  readonly projectSlug: string;
  readonly environmentSlug: string;
};

type ManualPublication = {
  readonly environmentId: string;
  readonly actorId: string;
  readonly message: string | null;
  readonly review: EnvironmentPublicationReview;
};

const loadDestructiveEnvironmentSave = Effect.fn(
  "CloudDeployment.loadDestructiveEnvironmentSave",
)(function* (environmentId: string) {
  const database = yield* Database;
  const snapshotProjection = yield* loadEnvironmentSnapshotProjection({
    kind: "environment",
    environmentId,
  });
  const workingProjection = yield* database.transaction(
    loadCurrentEnvironmentSnapshotProjection(environmentId),
  );
  const explicitState = snapshotProjection.explicitStates.find(
    (state) => state.environmentId === environmentId,
  );
  return projectDestructiveEnvironmentSave({
    workingNodes: workingProjection.nodeSnapshots,
    savedNodes: explicitState?.saved?.nodes ?? [],
    appliedNodes: explicitState?.applied.nodes ?? [],
  });
});

/** Fresh machine testimony for every deployed Volume the Working State removes. */
export const prepareEnvironmentDestructiveVolumes = Effect.fn(
  "CloudDeployment.prepareEnvironmentDestructiveVolumes",
)(function* (actor: Actor, input: EnvironmentContextInput) {
  const context = yield* requireEnvironmentForActor(actor, input);
  const destructiveSave = yield* loadDestructiveEnvironmentSave(
    context.environment.id,
  );
  return yield* gatherExactTombstonedVolumeReviews({
    actor,
    organizationSlug: input.organizationSlug,
    environmentId: context.environment.id,
    resourceIds: destructiveSave.volumeIds,
  });
});

const requirePullableManualSdkDeploy = Effect.fn(
  "CloudDeployment.requirePullableManualSdkDeploy",
)(function* (environmentId: string) {
  const document = yield* loadEnvironmentDocument(environmentId);
  const error = findUnpullableSdkDeployImages(
    document.intent.services.map((node) => ({
      id: node.id,
      name: node.config.name,
      source: node.config.source,
    })),
  );
  if (error) return yield* error;
});

/**
 * Volume evidence is gathered from machines, so it runs before the
 * publication transaction. The reviewed set must match the current deployed
 * removals and their fresh testimony; a drift returns the fresh reviews so
 * the canvas can ask for confirmation again.
 */
const requireFreshDestructiveVolumeEvidence = Effect.fn(
  "CloudDeployment.requireFreshDestructiveVolumeEvidence",
)(function* (actor: Actor, environmentId: string, input: ReviewedPublicationInput) {
  const destructiveSave = yield* loadDestructiveEnvironmentSave(environmentId);
  if (destructiveSave.volumeIds.length === 0) return;
  const freshReviews = yield* gatherExactTombstonedVolumeReviews({
    actor,
    organizationSlug: input.organizationSlug,
    environmentId,
    resourceIds: destructiveSave.volumeIds,
  });
  const mismatch = getDestructiveVolumeReviewMismatch({
    reviewed: input.review.destructiveVolumeReviews,
    fresh: freshReviews,
  });
  if (mismatch !== null) {
    return yield* new DestructiveVolumeReviewChangedError({
      reason: "review_updated_evidence",
      message: mismatch,
      freshReviews,
    });
  }
});

/** Publishes the reviewed Working State and admits that exact Saved revision in the caller's transaction. */
export const createManualEnvironmentDeployment = Effect.fn(
  "CloudDeployment.createManualEnvironmentDeployment",
)(function* (input: ManualPublication) {
  const saved = yield* saveReviewedEnvironmentState({
    ...input,
    revisionPolicy: "reuse_latest_if_equivalent",
  });
  const deployment = yield* admitEnvironmentDeployment({
    environmentId: input.environmentId,
    triggerOrigin: { origin: "manual", actorId: input.actorId },
    message: input.message,
    savedStateSnapshotId: saved.savedStateSnapshotId,
    serviceActionPolicy: { kind: "all_affected_required" },
  });
  return {
    environmentDeploymentId: deployment.id,
    status: deployment.status,
    createdAt: deployment.createdAt,
  };
});

const publishReviewedEnvironment = Effect.fn(
  "CloudDeployment.publishReviewedEnvironment",
)(function* (actor: Actor, environmentId: string, input: ReviewedPublicationInput) {
  const publication: ManualPublication = {
    environmentId,
    actorId: actor.userId,
    message: input.message || null,
    review: input.review,
  };
  switch (input.intent) {
    case "save": {
      yield* saveReviewedEnvironmentState({
        ...publication,
        revisionPolicy: "always_create",
      });
      return { state: "saved" as const };
    }
    case "manual_deploy": {
      const deployment = yield* createManualEnvironmentDeployment(publication);
      return {
        state: "deployment_queued" as const,
        environmentDeploymentId: deployment.environmentDeploymentId,
      };
    }
    default: {
      const _never: never = input.intent;
      return _never;
    }
  }
});

/**
 * The one manual publication command. Save and Deploy run the same review
 * against current server state and fail before any write on a stale Working
 * fingerprint, stale Saved basis, or incomplete destructive set. Save
 * publishes only. Deploy publishes and admits in one transaction, then
 * dispatches after commit; a send failure leaves a failed attempt and the
 * Saved revision behind for an explicit retry.
 */
export const submitReviewedPublication = Effect.fn(
  "CloudDeployment.submitReviewedPublication",
)(function* (actor: Actor, input: ReviewedPublicationInput) {
  const context = yield* requireEnvironmentForActor(actor, input);
  const environmentId = context.environment.id;
  if (input.intent === "manual_deploy") {
    yield* requirePullableManualSdkDeploy(environmentId);
  }
  yield* requireFreshDestructiveVolumeEvidence(actor, environmentId, input);
  const committed = yield* withMutationResult(
    publishReviewedEnvironment(actor, environmentId, input),
    { isolationLevel: "read committed" },
  ).pipe(
    Effect.catchIf(isActiveDeploymentUniqueViolation, () =>
      new Validation({
        field: "environmentId",
        message: "An environment deployment attempt is already active.",
      }),
    ),
  );
  const outcome = committed.data;
  if (outcome.state !== "deployment_queued") return outcome;
  return yield* dispatchEnvironmentDeployment({
    environmentDeploymentId: outcome.environmentDeploymentId,
    environmentId,
  }).pipe(
    Effect.map((): EnvironmentPublicationSubmissionOutcome => outcome),
    Effect.catchTag(
      "InngestEventSendError",
      (): Effect.Effect<EnvironmentPublicationSubmissionOutcome> =>
        Effect.succeed({
          state: "attempt_dispatch_failed",
          environmentDeploymentId: outcome.environmentDeploymentId,
        }),
    ),
  );
});
