import "@tanstack/react-start/server-only";

import { Effect } from "effect";
import { lockEnvironmentDeploymentQueue } from "#/modules/deployments/queue-lock.server";
import type { ReviewedEnvironmentPublication } from "#/modules/environment-design/working-state-review";
import { publishReviewedWorkingState } from "#/modules/environment-design/saved-state-operations.server";
import {
  admitEnvironmentDeployment,
  assertManualDeploymentQueueVacant,
} from "#/modules/deployments/admission.server";

export const createManualEnvironmentDeployment = Effect.fn(
  "Deployments.createManualEnvironmentDeployment",
)(function* (input: {
  readonly environmentId: string;
  readonly actorId: string;
  readonly message: string | null;
  readonly review: ReviewedEnvironmentPublication;
}) {
  yield* lockEnvironmentDeploymentQueue(input.environmentId);
  yield* assertManualDeploymentQueueVacant(input.environmentId);
  const saved = yield* publishReviewedWorkingState({
    environmentId: input.environmentId,
    actorId: input.actorId,
    message: input.message,
    review: input.review,
    revisionPolicy: "reuse_latest_if_equivalent",
    staleWorkingMessage:
      "Working State changed after the manual deployment was reviewed.",
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
    serviceCount: saved.nodeSnapshots.filter(
      ({ nodeType }) => nodeType === "service",
    ).length,
  };
});
