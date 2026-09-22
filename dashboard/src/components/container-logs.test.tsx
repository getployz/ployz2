// @vitest-environment jsdom
import { StrictMode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { DbProvider } from "@tanstack/react-db";
import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterContextProvider } from "@tanstack/react-router";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { getDbClient } from "#/collections/scope";
import { ContainerLogs } from "./container-logs";

it("loads history after Strict Mode remount and closes viewer-owned streams", async () => {
  const sources: FakeEventSource[] = [];
  class FakeEventSource extends EventTarget {
    closed = false;
    constructor() { super(); sources.push(this); }
    close() { this.closed = true; }
  }
  vi.stubGlobal("EventSource", FakeEventSource);
  const fetchHistory = vi.fn(async (_url: string, init: RequestInit) => {
    expect(init.signal?.aborted).toBe(false);
    return Response.json({ records: [], errors: [] });
  });
  vi.stubGlobal("fetch", fetchHistory);
  const client = new QueryClient();
  const root = createRootRoute();
  const protectedRoute = createRoute({ getParentRoute: () => root, id: "_protected", beforeLoad: () => ({ session: { session: { id: "session" }, user: { id: "user" } } }) });
  const index = createRoute({ getParentRoute: () => protectedRoute, path: "/" });
  const router = createRouter({ routeTree: root.addChildren([protectedRoute.addChildren([index])]), history: createMemoryHistory({ initialEntries: ["/"] }) });
  await router.load();
  try {
    render(<StrictMode><QueryClientProvider client={client}><DbProvider client={getDbClient(client)}><RouterContextProvider router={router}>
      <ContainerLogs selection={{ organizationSlug: "acme", environmentSlug: "production" }} />
    </RouterContextProvider></DbProvider></QueryClientProvider></StrictMode>);
    const source = sources.at(-1);
    if (!source) throw new Error("Viewer did not open its log stream");
    await act(async () => source.dispatchEvent(new MessageEvent("log", { data: JSON.stringify({ type: "record", record: {
      id: "m/c/100/0", timestamp: "100", machineId: "m", machineName: "Server", containerId: "c", serviceName: "api", channel: "stdout", message: "hello",
    } }) })));
    const button = screen.getByRole("button", { name: "Load older" });
    await waitFor(() => expect(button.hasAttribute("disabled")).toBe(false));
    fireEvent.click(button);
    await waitFor(() => expect(fetchHistory).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(button.hasAttribute("disabled")).toBe(true));
  } finally {
    cleanup();
    expect(sources.every(source => source.closed)).toBe(true);
    await getDbClient(client).cleanup(); client.clear(); vi.unstubAllGlobals();
  }
});
