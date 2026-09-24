import { setTimeout as sleep } from "node:timers/promises";
import { Effect, Schema } from "effect";
import { changeCursorSchema, type ChangeName } from "#/collections/read.contract";
import { eventStreamResponse, sseEvent } from "#/server/event-stream";
import { publicErrorResponse } from "#/server/public-error";

export type OrgChangesHandlerDeps = {
  /** Refuses non-members. */
  authorize: (organizationSlug: string) => Promise<{ organizationId: string }>;
  /** Where a stream without a resume point starts: the current horizon. */
  currentCursor: () => Promise<string>;
  /** Collections changed since `since`, and the cursor to read from next. */
  readChanges: (input: { organizationId: string; since: string }) => Promise<{
    cursor: string;
    /** Retention deleted changes after `since`. */
    expired: boolean;
    collections: ChangeName[];
  }>;
};

const POLL_MS = 250;
const isCursor = Schema.is(changeCursorSchema);

/**
 * Organization change stream: each event's id is its cursor and its data names the collections to refetch.
 * Resuming from a Last-Event-ID that retention has passed sends `reset`: refetch every collection.
 */
export async function handleOrgChangesRequest(request: Request, organizationSlug: string, deps: OrgChangesHandlerDeps) {
  let organizationId: string;
  try {
    ({ organizationId } = await deps.authorize(organizationSlug));
  } catch (cause) {
    return publicErrorResponse(cause);
  }
  const lastEventId = request.headers.get("Last-Event-ID");
  const resuming = isCursor(lastEventId);
  // Taken before the response opens: the client refetches on `open`, so every change at or after
  // this cursor is either in that refetch or announced by the stream.
  let cursor: string;
  try {
    cursor = resuming ? lastEventId : await deps.currentCursor();
  } catch (cause) {
    return publicErrorResponse(cause);
  }
  return eventStreamResponse(request.signal, (signal) => orgChangeEvents(organizationId, cursor, resuming, deps, signal),
    { heartbeatMs: 15_000, retryMs: 1000 });
}

// ponytail: one poll loop per open stream; move to one loop per instance if open tabs reach the thousands.
async function* orgChangeEvents(organizationId: string, start: string, resume: boolean, deps: OrgChangesHandlerDeps, signal: AbortSignal) {
  let cursor = start;
  // Only the resume can be expired; a live cursor is always recent, and an empty log would reset every poll.
  let resuming = resume;
  while (!signal.aborted) {
    const changes = await deps.readChanges({ organizationId, since: cursor }).catch((cause: unknown) => {
      Effect.runFork(Effect.logError("Organization change log read failed.", cause));
      return undefined;
    });
    // Ending the stream on a failed read makes EventSource reconnect from its last event id.
    if (!changes) return;
    if (resuming && changes.expired) {
      yield sseEvent({ id: changes.cursor, event: "reset", data: {} });
    } else if (changes.collections.length > 0) {
      yield sseEvent({ id: changes.cursor, event: "changes", data: { collections: changes.collections } });
    }
    cursor = changes.cursor;
    resuming = false;
    await sleep(POLL_MS, undefined, { signal }).catch(() => {});
  }
}
