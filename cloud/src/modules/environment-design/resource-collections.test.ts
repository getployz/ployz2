import { QueryClient } from "@tanstack/react-query";
import { createApiCollection, preloadCollection } from "#/collections/query-collection";
import { expect, it, vi } from "vitest";
import { createCollection, localOnlyCollectionOptions } from "@tanstack/react-db";
import { createVolumeResourcesCollection } from "./resource-collections";
import { compileEnvironmentIntent, type SavedEnvironmentIntent } from "@ployz/sdk/config";
import type { Collection } from "@tanstack/react-db";
type Sources = Parameters<typeof createVolumeResourcesCollection>[0]["sources"];
type Row<Key extends keyof Sources> = Sources[Key] extends Collection<infer R, infer _K, infer _U, infer _S, infer _I> ? R : never;

function createStores() {
  const collection = <T extends { id: string }>(initialData: T[]) => createCollection(localOnlyCollectionOptions<T>({ getKey: (row) => row.id, initialData }));
  const environmentId = "00000000-0000-4000-8000-000000000001";
  const projectId = "00000000-0000-4000-8000-000000000002";
  const organizationId = "00000000-0000-4000-8000-000000000003";
  const id = "00000000-0000-4000-8000-000000000004";
  const lineageId = "00000000-0000-4000-8000-000000000005";
  const now = new Date("2026-09-08T00:00:00Z");
  const intent: SavedEnvironmentIntent = { version: 1, environmentSlug: "production", services: [], variableGroups: [], volumes: [{ resourceId: id, resourceLineageId: lineageId, name: "data" }] };
  const sources: Sources = {
    resources: collection<Row<"resources">>([{ id, environmentId, projectId, organizationId, lineageId, implementationType: "volume", variableGroupId: null, createdAt: now, updatedAt: now }]),
    lineages: collection<Row<"lineages">>([{ id: lineageId, projectId, organizationId, canonicalName: "data", canonicalSlug: "data", createdAt: now, updatedAt: now }]),
    documents: collection<Row<"documents">>([{ id: environmentId, projectId, organizationId, projectSlug: "app", namespace: "production", name: "Production", revision: "00000000-0000-4000-8000-000000000099", createdAt: now, updatedAt: now, intent, compiled: compileEnvironmentIntent(environmentId, intent) }]),
    positions: collection<Row<"positions">>([{ id: "00000000-0000-4000-8000-000000000006", environmentId, organizationId, resourceType: "volume", resourceId: id, x: 10, y: 20, createdAt: now, updatedAt: now }]),
    snapshots: collection<Row<"snapshots">>([]),
    removals: collection<Row<"removals">>([]),
  };
  return { sources, id, environmentId };
}

it("loads an authored volume with its Electric canvas position and no runtime history", async () => {
  const stores = createStores();
  const volumes = createVolumeResourcesCollection({ organizationSlug: "test", sources: stores.sources });
  await volumes.preload();
  expect(volumes.get(stores.id)).toMatchObject({ resource: { name: "data" }, isAuthored: true, canvasPosition: { x: 10, y: 20 } });
  expect(volumes.get(stores.id)?.canvasPosition).not.toHaveProperty("organizationId");
  stores.sources.documents.update(stores.environmentId, (document) => {
    const volume = document.intent.volumes.find((volume) => volume.resourceId === stores.id);
    if (volume) volume.name = "renamed-data";
  });
  await vi.waitFor(() => expect(volumes.get(stores.id)?.resource.name).toBe("renamed-data"));
  await volumes.cleanup();
});

it("updates joined volume history from API snapshots and completed removals", async () => {
  const { sources, id, environmentId } = createStores();
  const client = new QueryClient();
  const resource = sources.resources.get(id);
  if (!resource) throw new Error("Missing fixture resource");
  const now = resource.createdAt;
  let snapshots: Row<"snapshots">[] = [{ id: "snapshot", organizationId: resource.organizationId,
    environmentId, environmentDeploymentId: "deployment", nodeType: "volume", nodeId: id,
    nodeLineageId: resource.lineageId, configVersion: 1, config: { version: 2, name: "deployed-data" }, createdAt: now, updatedAt: now }];
  let removals: Row<"removals">[] = [];
  const snapshotCollection = createApiCollection({ queryClient: client, queryKey: ["history", "snapshots"],
    queryFn: async () => snapshots, getKey: (row: Row<"snapshots">) => row.id });
  const removalCollection = createApiCollection({ queryClient: client, queryKey: ["history", "removals"],
    queryFn: async () => removals, getKey: (row: Row<"removals">) => row.id });
  await Promise.all([preloadCollection(snapshotCollection), preloadCollection(removalCollection)]);
  sources.documents.update(environmentId, (document) => { document.intent.volumes = []; });
  const volumes = createVolumeResourcesCollection({ organizationSlug: "history-test",
    sources: { ...sources, snapshots: snapshotCollection, removals: removalCollection } });
  try {
    await volumes.preload();
    expect(volumes.get(id)).toMatchObject({ isAuthored: false, resource: { name: "deployed-data" } });
    removals = [{ id: "removal", organizationId: resource.organizationId, requestedByUserId: "user", environmentId,
      environmentDeploymentId: "deployment", environmentResourceId: id, retryOfAttemptId: null, volumes: [],
      status: "completed", inngestRunId: null, outcome: null, failureMessage: null, startedAt: now,
      terminalAt: new Date(now.getTime() + 1), createdAt: now, updatedAt: now }];
    await removalCollection.utils.refetch({ throwOnError: true });
    await vi.waitFor(() => expect(volumes.get(id)).toBeUndefined());
    const previous = snapshots[0];
    if (!previous) throw new Error("Missing fixture snapshot");
    snapshots = [{ ...previous, id: "new-snapshot", config: { version: 2, name: "redeployed-data" }, createdAt: new Date(now.getTime() + 2) }];
    await snapshotCollection.utils.refetch({ throwOnError: true });
    await vi.waitFor(() => expect(volumes.get(id)?.resource.name).toBe("redeployed-data"));
  } finally {
    await volumes.cleanup();
    await snapshotCollection.cleanup();
    await removalCollection.cleanup();
    client.clear();
  }
});
