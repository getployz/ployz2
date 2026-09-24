/**
 * Serves server-sent events as they are pulled from `events`, so a slow client applies backpressure.
 * Cancelling the response calls `onCancel` (wake the producer), then returns the iterator.
 */
export function eventStreamResponse(events: AsyncIterable<string>, onCancel?: () => void) {
  const iterator = events[Symbol.asyncIterator]();
  const encoder = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    async pull(controller) {
      const next = await iterator.next();
      if (next.done) controller.close();
      else controller.enqueue(encoder.encode(next.value));
    },
    async cancel() {
      onCancel?.();
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
