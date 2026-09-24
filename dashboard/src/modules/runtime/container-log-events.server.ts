import { setTimeout as sleep } from "node:timers/promises";
import { projectContainerLog } from "./container-log.collection";
import type { LogEvent } from "@ployz/sdk";
import { eventStreamResponse } from "#/server/event-stream";

const HEARTBEAT_MS = 15_000;

/** Pull-driven delivery preserves every log record rather than coalescing watch snapshots. */
export function containerLogResponse(request: Request, events: AsyncIterable<LogEvent>, close: () => Promise<void>) {
  const cancelled = new AbortController();
  // Once: the stream can end, or be cancelled before it ever started.
  let released: Promise<void> | undefined;
  const release = () => released ??= close();
  return eventStreamResponse(containerLogEvents(events, release, AbortSignal.any([request.signal, cancelled.signal])), () => {
    cancelled.abort();
    void release();
  });
}

async function* containerLogEvents(events: AsyncIterable<LogEvent>, close: () => Promise<void>, signal: AbortSignal) {
  const iterator = events[Symbol.asyncIterator]();
  const next = () => iterator.next().then((value) => ({ value }), (error) => ({ error }));
  try {
    let pending = next();
    for (;;) {
      const tick = new AbortController();
      const heartbeat = sleep(HEARTBEAT_MS, "heartbeat" as const, { signal: AbortSignal.any([signal, tick.signal]) })
        .catch(() => "stopped" as const);
      const result = await Promise.race([pending, heartbeat]);
      tick.abort();
      if (signal.aborted || result === "stopped") return;
      if (result === "heartbeat") {
        yield ": heartbeat\n\n";
        continue;
      }
      if ("error" in result) {
        yield "event: unavailable\ndata: {}\n\n";
        return;
      }
      if (result.value.done) return;
      const event = result.value.value;
      yield `event: log\ndata: ${JSON.stringify(event.type === "record" ? { type: "record", record: projectContainerLog(event.record) } : event)}\n\n`;
      pending = next();
    }
  } finally {
    await close();
    await iterator.return?.();
  }
}
