import "@tanstack/react-start/server-only";

import { and, eq, isNotNull, isNull, ne } from "drizzle-orm";
import { Effect } from "effect";
import { environmentDeployment as schemaEnvironmentDeployment } from "#/modules/deployments/tables";
import { afterDatabaseCommit, Database } from "#/server/database.server";
import { failUndispatchedDeployment } from "./runtime-lifecycle.repository.server";
import { Conflict } from "#/server/public-error";
import {
  type InngestClient,
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

/**
 * Dispatches any committed queued row that is still unowned. Replays enqueue
 * deliberately: the event has a deterministic deployment ID, so Inngest owns
 * deduplication across a crash after admission commit or after event send.
 */
const sendEnvironmentDeployment = Effect.fn(
  "Deployments.dispatchEnvironmentDeployment",
)(function* (input: EnvironmentDeploymentDispatchInput) {
  const { drizzle } = yield* Database;
  // The pending attempt waits for the building attempt to leave queued; dispatchPendingDeployment sends it then.
  const [building] = yield* drizzle.select({ id: schemaEnvironmentDeployment.id }).from(schemaEnvironmentDeployment)
    .where(and(eq(schemaEnvironmentDeployment.environmentId, input.environmentId),
      eq(schemaEnvironmentDeployment.status, "queued"), isNotNull(schemaEnvironmentDeployment.inngestRunId),
      ne(schemaEnvironmentDeployment.id, input.environmentDeploymentId)))
    .limit(1);
  if (building) return { state: "pending" as const };
  const requestedAt = new Date();
  const requested = yield* drizzle
    .update(schemaEnvironmentDeployment)
    .set({ dispatchRequestedAt: requestedAt, updatedAt: requestedAt })
    .where(
      and(
        eq(
          schemaEnvironmentDeployment.id,
          input.environmentDeploymentId,
        ),
        eq(schemaEnvironmentDeployment.environmentId, input.environmentId),
        eq(schemaEnvironmentDeployment.status, "queued"),
        isNull(schemaEnvironmentDeployment.inngestRunId),
      ),
    )
    .returning({ id: schemaEnvironmentDeployment.id });
  if (requested.length === 0) {
    return yield* new Conflict({
      message: "The queued deployment is no longer available to dispatch.",
    });
  }

  yield* sendInngestEvent(createEnvironmentDeployRequestedEvent(input)).pipe(
    Effect.catchTag("InngestEventSendError", (failure) =>
      Effect.gen(function* () {
        const failed = yield* failUndispatchedDeployment({
          environmentDeploymentId: input.environmentDeploymentId,
          failureCode: ENVIRONMENT_DEPLOYMENT_DISPATCH_FAILURE_CODE,
          message: ENVIRONMENT_DEPLOYMENT_DISPATCH_FAILURE_MESSAGE,
        });
        if (failed) return yield* failure;
      }),
    ),
  );
  return { state: "dispatched" as const };
});

/**
 * Sends the Environment's pending attempt, if any, once no building attempt holds the queue.
 * Typed explicitly: runtime-lifecycle calls this and this module calls runtime-lifecycle.
 */
export const dispatchPendingDeployment = (environmentId: string): Effect.Effect<void, never, Database | InngestClient> =>
  Effect.gen(function* () {
    const { drizzle } = yield* Database;
    const [pending] = yield* drizzle.select({ id: schemaEnvironmentDeployment.id }).from(schemaEnvironmentDeployment)
      .where(and(eq(schemaEnvironmentDeployment.environmentId, environmentId),
        eq(schemaEnvironmentDeployment.status, "queued"), isNull(schemaEnvironmentDeployment.inngestRunId)))
      .limit(1);
    if (pending) yield* sendEnvironmentDeployment({ environmentDeploymentId: pending.id, environmentId });
  }).pipe(
    Effect.catch((error) => Effect.logError("Failed to dispatch the pending deployment", error)),
    Effect.annotateLogs({ environmentId }),
    Effect.withSpan("Deployments.dispatchPendingDeployment"),
  );

export const dispatchEnvironmentDeployment = Effect.fn("Deployments.dispatchAfterCommit")(
  (input: EnvironmentDeploymentDispatchInput) => afterDatabaseCommit(sendEnvironmentDeployment(input)).pipe(
    Effect.as({ state: "dispatched" as const }),
  ),
);
