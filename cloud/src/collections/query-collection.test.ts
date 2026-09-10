// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { focusManager, onlineManager, QueryClient } from "@tanstack/react-query";
import { createApiCollection } from "./query-collection";

afterEach(() => {
  vi.useRealTimers();
  focusManager.setFocused(undefined);
  onlineManager.setOnline(true);
});

it("refreshes active snapshots, retains failed reads, and stops unused scopes", async () => {
  vi.useFakeTimers();
  focusManager.setFocused(true);
  const client = new QueryClient();
  client.mount();
  let rows = [{ id: "a", name: "before" }];
  const read = vi.fn(async () => rows);
  const collection = createApiCollection({
    queryClient: client, queryKey: ["test", "session"], queryFn: read,
    getKey: (row: { id: string; name: string }) => row.id,
  });
  const subscription = collection.subscribeChanges(() => {});
  await vi.advanceTimersByTimeAsync(10);
  expect(collection.get("a")?.name).toBe("before");
  rows = [{ id: "b", name: "after" }];
  await vi.advanceTimersByTimeAsync(15_000);
  expect(collection.get("a")).toBeUndefined();
  expect(collection.get("b")?.name).toBe("after");
  read.mockRejectedValueOnce(new Error("unavailable"));
  await vi.advanceTimersByTimeAsync(15_000);
  expect(collection.get("b")?.name).toBe("after");
  expect(collection.utils.isError).toBe(true);
  focusManager.setFocused(false);
  const hiddenReads = read.mock.calls.length;
  rows = [{ id: "c", name: "focused" }];
  await vi.advanceTimersByTimeAsync(30_000);
  expect(read).toHaveBeenCalledTimes(hiddenReads);
  focusManager.setFocused(true);
  await vi.advanceTimersByTimeAsync(10);
  expect(collection.get("c")?.name).toBe("focused");
  onlineManager.setOnline(false);
  rows = [];
  onlineManager.setOnline(true);
  await vi.advanceTimersByTimeAsync(10);
  expect(collection.size).toBe(0);
  subscription.unsubscribe();
  await vi.advanceTimersByTimeAsync(2_000);
  const inactiveReads = read.mock.calls.length;
  await vi.advanceTimersByTimeAsync(30_000);
  expect(read).toHaveBeenCalledTimes(inactiveReads);
  await collection.cleanup();
  client.unmount();
  client.clear();
});
