import { Effect, Schema } from "effect";
import { changeCursorSchema, type ChangeName } from "#/collections/read.contract";
import { publicErrorResponse } from "#/server/public-error";

export type OrgChangesHandlerDeps = {
  /** Refuses non-members. */
  authorize: (input: { request: Request; organizationSlug: string }) => Promise<{ organizationId: string }>;
  /** Collections changed since `since`, and the cursor to read from next. No `since` starts at now. */
  readChanges: (input: { organizationId: string; since: string | undefined }) => Promise<{
    cursor: string;
    /** Retention deleted changes after `since`. */
    expired: boolean;
    collections: ChangeName[];
  }>;
};

const POLL_MS = 250;
const PING_MS = 15_000;
const isCursor = Schema.is(changeCursorSchema);

/**
 * Organization change stream: each event's id is its cursor and its data names the collections to refetch.
 * Resuming from a Last-Event-ID that retention has passed sends `reset`: refetch every collection.
 */
export async function handleOrgChangesRequest(request: Request, organizationSlug: string, deps: OrgChangesHandlerDeps) {
  let organizationId: string;
  try {
    ({ organizationId } = await deps.authorize({ request, organizationSlug }));
  } catch (cause) {
    return publicErrorResponse(cause);
  }
  const lastEventId = request.headers.get("Last-Event-ID");
  const cancelled = new AbortController();
  const events = orgChangeEvents(organizationId, lastEventId !== null && isCursor(lastEventId) ? lastEventId : undefined, deps,
    AbortSignal.any([request.signal, cancelled.signal]));
  const encoder = new TextEncoder();
  const stream = new ReadableStream<Uint8Array>({
    async pull(controller) {
      const next = await events.next();
      if (next.done) controller.close();
      else controller.enqueue(encoder.encode(next.value));
    },
    cancel() {
      cancelled.abort();
    },
  });
  return new Response(stream, {
    headers: {
      "Cache-Control": "no-cache, no-transform",
      Connection: "keep-alive",
      "Content-Type": "text/event-stream",
      "X-Accel-Buffering": "no",
    },
  });
}

// ponytail: one poll loop per open stream; move to one loop per instance if open tabs reach the thousands.
async function* orgChangeEvents(organizationId: string, resumeFrom: string | undefined, deps: OrgChangesHandlerDeps, signal: AbortSignal) {
  yield "retry: 1000\n\n";
  let cursor = resumeFrom;
  // Only the resume can be expired; a live cursor is always recent, and an empty log would reset every poll.
  let resuming = cursor !== undefined;
  let lastWrite = Date.now();
  while (!signal.aborted) {
    let changes: Awaited<ReturnType<OrgChangesHandlerDeps["readChanges"]>>;
    try {
      changes = await deps.readChanges({ organizationId, since: cursor });
    } catch (cause) {
      // Ending the stream makes EventSource reconnect from its last event id.
      Effect.runFork(Effect.logError("Organization change log read failed.", cause));
      return;
    }
    if (resuming && changes.expired) {
      yield `id: ${changes.cursor}\nevent: reset\ndata: {}\n\n`;
      lastWrite = Date.now();
    } else if (changes.collections.length > 0) {
      yield `id: ${changes.cursor}\nevent: changes\ndata: ${JSON.stringify({ collections: changes.collections })}\n\n`;
      lastWrite = Date.now();
    } else if (Date.now() - lastWrite >= PING_MS) {
      yield ": ping\n\n";
      lastWrite = Date.now();
    }
    cursor = changes.cursor;
    resuming = false;
    await new Promise<void>((resolve) => {
      const wake = () => {
        clearTimeout(timer);
        signal.removeEventListener("abort", wake);
        resolve();
      };
      const timer = setTimeout(wake, POLL_MS);
      signal.addEventListener("abort", wake, { once: true });
    });
  }
}
