import { setTimeout as sleep } from "node:timers/promises";
import { afterEach, expect, it, vi } from "vitest";
import { handleOrgChangesRequest, type OrgChangesHandlerDeps } from "#/routes/api/org/-changes.handler";
import { NotFound, Unauthorized } from "#/server/public-error";

afterEach(() => {
  vi.useRealTimers();
});

function request(headers: Record<string, string> = {}) {
  return new Request("http://localhost/api/org/changes?organizationSlug=acme", { headers });
}

/** Reads a change stream as text. The stream is pulled, so it only polls while a test reads. */
function readEvents(response: Response) {
  if (!response.body) throw new Error("The change stream has no body");
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  const events = {
    text: "",
    /** Reads until `fragment` arrives or the stream ends. */
    async until(fragment: string) {
      while (!events.text.includes(fragment)) {
        const chunk = await reader.read();
        if (chunk.done) break;
        events.text += decoder.decode(chunk.value);
      }
      return events.text;
    },
    cancel: () => reader.cancel(),
  };
  return events;
}

it.each([
  ["anonymous", new Unauthorized(), 401],
  ["non-member", new NotFound({ message: "The organization was not found." }), 404],
])("refuses %s requests before reading the change log", async (_name, error, status) => {
  const deps = { authorize: vi.fn().mockRejectedValue(error), readChanges: vi.fn() };
  const response = await handleOrgChangesRequest(request(), "acme", deps);
  expect(response.status).toBe(status);
  expect(deps.readChanges).not.toHaveBeenCalled();
});

it("resumes after Last-Event-ID, names changed collections, pings, and disables proxy buffering", async () => {
  // Polls wait on real 250 ms timers (node:timers/promises); only the heartbeat timer is faked.
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const readChanges = vi.fn<OrgChangesHandlerDeps["readChanges"]>()
    .mockResolvedValueOnce({ cursor: "50", expired: false, collections: ["service"] })
    .mockResolvedValue({ cursor: "51", expired: false, collections: [] });
  const authorize = vi.fn().mockResolvedValue({ organizationId: "org-1" });
  const response = await handleOrgChangesRequest(request({ "Last-Event-ID": "42" }), "acme", { authorize, readChanges });

  expect(response.headers.get("Content-Type")).toBe("text/event-stream");
  expect(response.headers.get("X-Accel-Buffering")).toBe("no");
  expect(response.headers.get("Cache-Control")).toBe("private, no-store, no-transform");
  const events = readEvents(response);

  await events.until("event: changes");
  expect(readChanges).toHaveBeenNthCalledWith(1, { organizationId: "org-1", since: "42" });
  expect(events.text).toContain('id: 50\nevent: changes\ndata: {"collections":["service"]}\n\n');
  await vi.waitFor(() => expect(readChanges).toHaveBeenNthCalledWith(2, { organizationId: "org-1", since: "50" }));
  expect(events.text).not.toContain(": ping");
  await vi.advanceTimersByTimeAsync(15_000);
  await events.until(": ping\n\n");
  // Quiet polls advance the cursor without emitting events.
  expect(events.text.match(/event: changes/g)).toHaveLength(1);
  await vi.waitFor(() => expect(readChanges).toHaveBeenLastCalledWith({ organizationId: "org-1", since: "51" }));
  await events.cancel();
  const calls = readChanges.mock.calls.length;
  await sleep(600);
  expect(readChanges).toHaveBeenCalledTimes(calls);
});

it("starts at the current horizon without a valid Last-Event-ID", async () => {
  const readChanges = vi.fn<OrgChangesHandlerDeps["readChanges"]>().mockResolvedValue({ cursor: "7", expired: false, collections: [] });
  const response = await handleOrgChangesRequest(request({ "Last-Event-ID": "not-a-cursor" }), "acme", {
    authorize: vi.fn().mockResolvedValue({ organizationId: "org-1" }),
    readChanges,
  });
  const events = readEvents(response);
  // Reading the retry line lets the pulled stream poll.
  expect(await events.until("retry: 1000\n\n")).toBe("retry: 1000\n\n");
  await vi.waitFor(() => expect(readChanges).toHaveBeenCalledWith({ organizationId: "org-1", since: undefined }));
  await events.cancel();
});

it("sends reset when resuming below the retention fence, then streams changes normally", async () => {
  const readChanges = vi.fn<OrgChangesHandlerDeps["readChanges"]>()
    .mockResolvedValueOnce({ cursor: "90", expired: true, collections: ["service"] })
    // An empty log keeps reporting expiry; only the resume acts on it.
    .mockResolvedValueOnce({ cursor: "91", expired: true, collections: ["service"] })
    .mockResolvedValue({ cursor: "92", expired: true, collections: [] });
  const response = await handleOrgChangesRequest(request({ "Last-Event-ID": "3" }), "acme", {
    authorize: vi.fn().mockResolvedValue({ organizationId: "org-1" }),
    readChanges,
  });
  const events = readEvents(response);
  const text = await events.until('id: 91\nevent: changes\ndata: {"collections":["service"]}\n\n');
  await events.cancel();
  expect(text).toContain("id: 90\nevent: reset\ndata: {}\n\n");
  expect(text.match(/event: /g)).toHaveLength(2);
});

it("replays normally when resuming at or above the retention fence", async () => {
  const readChanges = vi.fn<OrgChangesHandlerDeps["readChanges"]>()
    .mockResolvedValueOnce({ cursor: "60", expired: false, collections: ["service"] })
    .mockResolvedValue({ cursor: "61", expired: false, collections: [] });
  const response = await handleOrgChangesRequest(request({ "Last-Event-ID": "55" }), "acme", {
    authorize: vi.fn().mockResolvedValue({ organizationId: "org-1" }),
    readChanges,
  });
  const events = readEvents(response);
  const text = await events.until("event: changes");
  await events.cancel();
  expect(text).not.toContain("event: reset");
  expect(text).toContain('id: 60\nevent: changes\ndata: {"collections":["service"]}\n\n');
});

it("ends the stream when the change log can't be read, so EventSource reconnects", async () => {
  const readChanges = vi.fn<OrgChangesHandlerDeps["readChanges"]>().mockRejectedValue(new Error("database down"));
  const response = await handleOrgChangesRequest(request(), "acme", {
    authorize: vi.fn().mockResolvedValue({ organizationId: "org-1" }),
    readChanges,
  });
  // The stream ends instead of reaching a changes event.
  expect(await readEvents(response).until("event: changes")).toBe("retry: 1000\n\n");
  expect(readChanges).toHaveBeenCalledOnce();
});
