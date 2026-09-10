import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import { getRawServicesCollection, getRawEnvironmentResourcesCollection, getCanvasPositionsCollection, getResourceLineagesCollection } from "#/electric/collections";
import { tableSyncUrl } from "#/electric/table-sync-url";

const TEST_BASE_URL = "http://localhost:61859/proxied/cloud/nick";

describe("Electric collection table-sync proxy URLs", () => {
  it("builds an absolute organization table-sync URL", () => {
    const value = tableSyncUrl(
      "environment_deployment",
      { organizationSlug: "nick" },
      TEST_BASE_URL,
    );

    expect(() => new URL(value)).not.toThrow();
    const url = new URL(value);
    expect(url.origin).toBe("http://localhost:61859");
    expect(url.pathname).toBe("/api/shapes/environment_deployment");
    expect(url.searchParams.get("organizationSlug")).toBe("nick");
  });

  it("builds an absolute user table-sync URL", () => {
    const url = new URL(tableSyncUrl("github_repository_cache", {}, TEST_BASE_URL));

    expect(url.origin).toBe("http://localhost:61859");
    expect(url.pathname).toBe("/api/shapes/github_repository_cache");
  });

  it("isolates node collections by authenticated scope while retaining automatic indexes", () => {
    const scope = { queryClient: new QueryClient(), sessionId: "session", userId: "user" };
    for (const get of [getRawServicesCollection, getRawEnvironmentResourcesCollection,
      getCanvasPositionsCollection, getResourceLineagesCollection]) {
      const collection = get("acme", scope);
      expect(collection.config.autoIndex).toBe("eager");
      expect(get("acme", scope)).toBe(collection);
      expect(get("other-org", scope)).not.toBe(collection);
      expect(get("acme", { ...scope, sessionId: "other-session" })).not.toBe(collection);
      expect(get("acme", { ...scope, userId: "other-user" })).not.toBe(collection);
      expect(get("acme", { ...scope, queryClient: new QueryClient() })).not.toBe(collection);
    }
  });
});
