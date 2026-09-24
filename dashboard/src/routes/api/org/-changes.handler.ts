import { Schema } from "effect";
import { changeCursorSchema } from "#/collections/read.contract";
import { publicErrorResponse, Validation } from "#/server/public-error";

export type OrgChangesHandlerDeps = {
  /** Refuses non-members. */
  authorize: (input: { request: Request; organizationSlug: string }) => Promise<{ organizationId: string }>;
  /** Collections changed since `since`, and the cursor to read from next. No `since` starts at now. */
  readChanges: (input: { organizationId: string; since: string | undefined }) => Promise<{
    cursor: string;
    collections: string[];
  }>;
};

const POLL_MS = 250;
const PING_MS = 15_000;
const isCursor = Schema.is(changeCursorSchema);

/** Organization change stream: each event's id is its cursor and its data names the collections to refetch. */
export async function handleOrgChangesRequest(request: Request, organizationSlug: string, deps: OrgChangesHandlerDeps) {
  let organizationId: string;
  try {
    ({ organizationId } = await deps.authorize({ request, organizationSlug }));
  } catch (cause) {
    return publicErrorResponse(cause);
  }
  const lastEventId = request.headers.get("Last-Event-ID");
  let cursor = lastEventId !== null && isCursor(lastEventId) ? lastEventId : undefined;
  const encoder = new TextEncoder();
  let stop = (_closeController: boolean) => {};
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      let closed = false;
      let pollTimer: ReturnType<typeof setTimeout> | undefined;
      const write = (chunk: string) => {
        if (!closed) controller.enqueue(encoder.encode(chunk));
      };
      const pingTimer = setInterval(() => write(": ping\n\n"), PING_MS);
      const abort = () => stop(true);
      stop = (closeController) => {
        if (closed) return;
        closed = true;
        clearTimeout(pollTimer);
        clearInterval(pingTimer);
        request.signal.removeEventListener("abort", abort);
        if (closeController) controller.close();
      };
      // ponytail: one poll loop per open stream; move to one loop per instance if open tabs reach the thousands.
      const poll = async () => {
        try {
          const changes = await deps.readChanges({ organizationId, since: cursor });
          if (changes.collections.length > 0) {
            write(`id: ${changes.cursor}\nevent: changes\ndata: ${JSON.stringify({ collections: changes.collections })}\n\n`);
          }
          cursor = changes.cursor;
        } catch (cause) {
          // EventSource reconnects from its last event id.
          console.error("[org-changes] change log read failed", cause);
          stop(true);
        }
        if (!closed) pollTimer = setTimeout(() => void poll(), POLL_MS);
      };
      request.signal.addEventListener("abort", abort, { once: true });
      write("retry: 1000\n\n");
      void poll();
    },
    cancel() {
      stop(false);
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

export function orgChangesValidationError() {
  return new Validation({ message: "A valid organization slug is required" });
}
