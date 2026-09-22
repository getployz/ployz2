import { expect, it, vi } from "vitest";
import { appendContainerLogs, createContainerLogs, historyBoundaries, remainingHistory, mergeContainerHistory, type ContainerLogRow } from "./container-log.collection";

const row = (containerId: string, timestamp: string, ordinal = 0): ContainerLogRow => ({
  id: `server/${containerId}/${timestamp}/${ordinal}`, timestamp,
  machineId: "server", machineName: "Server", containerId, serviceName: "api", channel: "stdout", message: "same message",
});
it("extends per-container history without clearing live output or collapsing identical records", async () => {
  const collection = createContainerLogs("log-test");
  const subscription = collection.subscribeChanges(() => undefined);
  try {
    await collection.preload();
    appendContainerLogs(collection, [row("busy", "100"), row("quiet", "10"), row("busy", "110")]);
    expect(historyBoundaries([...collection.values()])).toEqual({ "server/busy": "100", "server/quiet": "10" });
    mergeContainerHistory(collection, [row("busy", "99"), row("busy", "100"), row("busy", "100", 1), row("quiet", "9")]);
    expect(collection.size).toBe(6);
    expect(collection.has("server/busy/110/0")).toBe(true);
    expect(collection.has("server/busy/100/1")).toBe(true);
    expect(historyBoundaries([...collection.values()])).toEqual({ "server/busy": "99", "server/quiet": "9" });
  } finally {
    subscription.unsubscribe();
    await collection.cleanup();
  }
});

it("drops buffered rows after the last subscriber leaves", async () => {
  const collection = createContainerLogs("log-test-lifetime");
  const subscription = collection.subscribeChanges(() => undefined);
  try {
    await collection.preload();
    appendContainerLogs(collection, [row("busy", "100")]);
    expect([...collection.values()].map((record) => record.timestamp)).toEqual(["100"]);
    subscription.unsubscribe();
    await vi.waitFor(() => {
      expect([...collection.values()]).toEqual([]);
    });
  } finally {
    await collection.cleanup();
  }
});

it("new sources remain pageable after earlier sources exhaust their history", () => {
  const exhausted = { "server/a": "10" };
  expect(remainingHistory([row("a", "10"), row("b", "100")], exhausted)).toEqual({ "server/b": "100" });
});
