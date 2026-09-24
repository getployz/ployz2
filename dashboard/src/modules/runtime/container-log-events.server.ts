import { projectContainerLog } from "./container-log.collection";
import type { LogEvent } from "@ployz/sdk";
import { eventStreamResponse, sseEvent } from "#/server/event-stream";

/** Pull-driven delivery preserves every log record rather than coalescing watch snapshots. */
export function containerLogResponse(request: Request, events: AsyncIterable<LogEvent>, close: () => Promise<void>) {
  const iterator = events[Symbol.asyncIterator]();
  // Once: the stream can end, or be cancelled before it ever started.
  let released: Promise<void> | undefined;
  const release = () => released ??= close().then(async () => { await iterator.return?.(); });
  return eventStreamResponse(request.signal, (signal) => {
    signal.addEventListener("abort", () => void release(), { once: true });
    return containerLogEvents(iterator, release, signal);
  }, { heartbeatMs: 15_000 });
}

async function* containerLogEvents(iterator: AsyncIterator<LogEvent>, release: () => Promise<void>, signal: AbortSignal) {
  const stopped = new Promise<undefined>((resolve) => signal.addEventListener("abort", () => resolve(undefined), { once: true }));
  try {
    while (!signal.aborted) {
      const result = await Promise.race([iterator.next().then((value) => ({ value }), (error) => ({ error })), stopped]);
      if (!result) return;
      if ("error" in result) {
        yield sseEvent({ event: "unavailable", data: {} });
        return;
      }
      if (result.value.done) return;
      const event = result.value.value;
      yield sseEvent({ event: "log", data: event.type === "record" ? { type: "record", record: projectContainerLog(event.record) } : event });
    }
  } finally {
    await release();
  }
}
