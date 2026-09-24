/** One server-sent event frame. */
export function sseEvent(input: { id?: string; event: string; data: object }) {
  return `${input.id === undefined ? "" : `id: ${input.id}\n`}event: ${input.event}\ndata: ${JSON.stringify(input.data)}\n\n`;
}

/**
 * Serves server-sent events as they are pulled from `events`, so a slow client applies backpressure.
 * `events` gets a signal that aborts when the request ends or the client cancels. When no event
 * arrives for `heartbeatMs`, a `: ping` comment keeps proxies from closing the idle stream; the
 * pending event carries over to the next pull.
 */
export function eventStreamResponse(
  requestSignal: AbortSignal,
  events: (signal: AbortSignal) => AsyncIterable<string>,
  options: { heartbeatMs: number; retryMs?: number },
) {
  const cancelled = new AbortController();
  const iterator = events(AbortSignal.any([requestSignal, cancelled.signal]))[Symbol.asyncIterator]();
  const encoder = new TextEncoder();
  let pending: Promise<IteratorResult<string>> | undefined;
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      if (options.retryMs !== undefined) controller.enqueue(encoder.encode(`retry: ${options.retryMs}\n\n`));
    },
    async pull(controller) {
      pending ??= iterator.next();
      let timer: ReturnType<typeof setTimeout> | undefined;
      const heartbeat = new Promise<"heartbeat">((resolve) => {
        timer = setTimeout(() => resolve("heartbeat"), options.heartbeatMs);
      });
      const next = await Promise.race([pending, heartbeat]).finally(() => clearTimeout(timer));
      if (next === "heartbeat") {
        controller.enqueue(encoder.encode(": ping\n\n"));
        return;
      }
      pending = undefined;
      if (next.done) controller.close();
      else controller.enqueue(encoder.encode(next.value));
    },
    async cancel() {
      cancelled.abort();
      await iterator.return?.();
    },
  });
  return new Response(body, {
    headers: {
      "Cache-Control": "private, no-store, no-transform",
      "Content-Type": "text/event-stream",
      "X-Accel-Buffering": "no",
    },
  });
}
