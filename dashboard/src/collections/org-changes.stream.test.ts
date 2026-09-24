// @vitest-environment jsdom
import { QueryClient } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { changeCollections } from "./collections";
import { watchOrganizationChanges } from "./org-changes.stream";
import { getDbClient } from "./scope";

class FakeEventSource extends EventTarget {
  static latest: FakeEventSource | undefined;
  constructor() { super(); FakeEventSource.latest = this; }
  close() {}
}

afterEach(() => {
  vi.unstubAllGlobals();
});

it("refetches every change-log collection on reset", async () => {
  vi.stubGlobal("EventSource", FakeEventSource);
  const queryClient = new QueryClient();
  const scope = { queryClient, sessionId: "session", userId: "user" };
  const refetches = Object.values(changeCollections).map((get) => vi.spyOn(get("acme", scope).utils, "refetch").mockResolvedValue([]));
  const stop = watchOrganizationChanges("acme", scope);
  try {
    FakeEventSource.latest?.dispatchEvent(new MessageEvent("reset", { data: "{}" }));
    for (const refetch of refetches) expect(refetch).toHaveBeenCalledOnce();
  } finally {
    stop();
    await getDbClient(queryClient).cleanup();
    queryClient.clear();
  }
});
