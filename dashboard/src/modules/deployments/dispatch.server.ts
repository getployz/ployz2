import "@tanstack/react-start/server-only";

import { and, eq, isNull } from "drizzle-orm";
import { Effect } from "effect";
import { environmentDeployment as schemaEnvironmentDeployment } from "#/modules/deployments/tables";
import { failAwaitingVolumeRemoveAttemptsForDeploymentInTransaction } from "#/modules/runtime/volume-removal.repository";
import { Database } from "#/server/database.server";
import { Conflict } from "#/server/public-error";
import {
  sendInngestEvent,
} from "#/modules/inngest/client";
import { createEnvironmentDeployRequestedEvent } from "#/modules/inngest/events";

export const ENVIRONMENT_DEPLOYMENT_DISPATCH_FAILURE_CODE =
  "inngest_dispatch_failed";
export const ENVIRONMENT_DEPLOYMENT_DISPATCH_FAILURE_MESSAGE =
  "Cloud could not dispatch the deployment workflow.";

export type EnvironmentDeploymentDispatchInput = {
  readonly environmentDeploymentId: string;
  readonly environmentId: string;
};

function unownedQueuedAttempt(input: EnvironmentDeploymentDispatchInput) {
  return and(
    eq(schemaEnvironmentDeployment.id, input.environmentDeploymentId),
    eq(schemaEnvironmentDeployment.environmentId, input.environmentId),
    eq(schemaEnvironmentDeployment.status, "queued"),
    isNull(schemaEnvironmentDeployment.inngestRunId),
  );
}

/**
 * Fails the attempt and its linked awaiting Volume removals together, but
 * only while no worker owns the row. A claimed row means the event reached
 * Inngest despite the send error, so its owner and Volume rows stay intact.
 */
const terminalizeUnownedDispatchFailure = Effect.fn(
  "Deployments.terminalizeUnownedDispatchFailure",
)(function* (input: EnvironmentDeploymentDispatchInput) {
  const database = yield* Database;
  return yield* database.transaction(
    Effect.gen(function* () {
      const tx = (yield* Database).drizzle;
      const finishedAt = new Date();
      const failed = yield* tx
        .update(schemaEnvironmentDeployment)
        .set({
          status: "failed",
          failureCode: ENVIRONMENT_DEPLOYMENT_DISPATCH_FAILURE_CODE,
          failureMessage: ENVIRONMENT_DEPLOYMENT_DISPATCH_FAILURE_MESSAGE,
          finishedAt,
          updatedAt: finishedAt,
        })
        .where(unownedQueuedAttempt(input))
        .returning({ id: schemaEnvironmentDeployment.id });
      if (failed.length === 0) return { state: "already_claimed" as const };
      yield* failAwaitingVolumeRemoveAttemptsForDeploymentInTransaction(tx, {
        environmentDeploymentId: input.environmentDeploymentId,
        deploymentDisposition: "failed",
        now: finishedAt,
      });
      return { state: "failed" as const };
    }),
  );
});

/**
 * Dispatches any committed queued row that is still unowned. Replays enqueue
 * deliberately: the event has a deterministic deployment ID, so Inngest owns
 * deduplication across a crash after admission commit or after event send.
 */
export const dispatchEnvironmentDeployment = Effect.fn(
  "Deployments.dispatchEnvironmentDeployment",
)(function* (input: EnvironmentDeploymentDispatchInput) {
  const { drizzle } = yield* Database;
  const requestedAt = new Date();
  const requested = yield* drizzle
    .update(schemaEnvironmentDeployment)
    .set({ dispatchRequestedAt: requestedAt, updatedAt: requestedAt })
    .where(unownedQueuedAttempt(input))
    .returning({ id: schemaEnvironmentDeployment.id });
  if (requested.length === 0) {
    return yield* new Conflict({
      message: "The queued deployment is no longer available to dispatch.",
    });
  }

  return yield* sendInngestEvent(createEnvironmentDeployRequestedEvent(input)).pipe(
    Effect.map(() => ({ state: "dispatched" as const })),
    Effect.catchTag("InngestEventSendError", (failure) =>
      Effect.gen(function* () {
        const outcome = yield* terminalizeUnownedDispatchFailure(input);
        if (outcome.state === "already_claimed") return outcome;
        return yield* failure;
      }),
    ),
  );
});
