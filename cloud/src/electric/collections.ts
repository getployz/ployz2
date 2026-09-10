import { createApiCollection } from "#/collections/query-collection";
import { readCollectionServerFn } from "#/collections/read.functions";
import { cachedByCollectionScope } from "#/collections/scope";
import { snakeCamelMapper, type Row } from "@electric-sql/client";
import {
  electricCollectionOptions,
  type ElectricCollectionConfig,
} from "@tanstack/electric-db-collection";
import {
  BasicIndex,
  createCollection,
} from "@tanstack/react-db";

import { tableSyncUrl } from "#/electric/table-sync-url";
import type { OrganizationTableName } from "#/electric/synced-tables.server";
import {
  environmentDeployment as schemaEnvironmentDeployment,
  environmentSavedStateSnapshot as schemaEnvironmentSavedStateSnapshot,
} from "#/modules/deployments/tables";
import {
  service as schemaService,
  environmentCanvasNodePosition as schemaEnvironmentCanvasNodePosition,
  resourceLineage as schemaResourceLineage,
  environmentResource as schemaEnvironmentResource,
} from "#/modules/environment-design/tables";
import {
  project as schemaProject,
  environment as schemaEnvironment,
} from "#/modules/project/tables";
import {
  environmentNodeConfigSnapshot as schemaEnvironmentNodeConfigSnapshot,
  environmentNodeIntroduction as schemaEnvironmentNodeIntroduction,
  volumeRemoveAttempt as schemaVolumeRemoveAttempt,
} from "#/modules/runtime/tables";

type ProjectRow = typeof schemaProject.$inferSelect;
type EnvironmentRow = typeof schemaEnvironment.$inferSelect;
type ServiceRow = typeof schemaService.$inferSelect;
type CanvasPositionRow = typeof schemaEnvironmentCanvasNodePosition.$inferSelect;
type ResourceLineageRow = typeof schemaResourceLineage.$inferSelect;
type EnvironmentResourceRow = typeof schemaEnvironmentResource.$inferSelect;
type EnvironmentDeploymentRow = typeof schemaEnvironmentDeployment.$inferSelect;
type EnvironmentSavedStateRevisionRow = Pick<
  typeof schemaEnvironmentSavedStateSnapshot.$inferSelect,
  "id" | "organizationId" | "environmentId"
>;
type EnvironmentNodeConfigSnapshotRow =
  typeof schemaEnvironmentNodeConfigSnapshot.$inferSelect;
type EnvironmentNodeIntroductionRow =
  typeof schemaEnvironmentNodeIntroduction.$inferSelect;
type VolumeRemoveAttemptRow = typeof schemaVolumeRemoveAttempt.$inferSelect;

const dateParser = (value: string) => new Date(value);
const parser = {
  timestamptz: dateParser,
  timestamp: dateParser,
  int8: (value: string) => Number(value),
};

function makeOrganizationCollection<T extends Row<Date>>(input: {
  table: OrganizationTableName;
  organizationSlug: string;
  baseUrl?: string;
  getKey: (row: T) => string | number;
}) {
  const id = `electric:${input.organizationSlug}:${input.table}`;
  const config: ElectricCollectionConfig<T> = {
    id,
    startSync: true,
    ["shapeOptions"]: {
      url: tableSyncUrl(
        input.table,
        { organizationSlug: input.organizationSlug },
        input.baseUrl,
      ),
      columnMapper: snakeCamelMapper(),
      // SAFETY: Electric's parser is row-generic; these handlers are column-type parsers shared across tables.
      parser: parser as typeof parser &
        NonNullable<ElectricCollectionConfig<T>["shapeOptions"]["parser"]>,
    },
    getKey: input.getKey,
    autoIndex: "eager",
    defaultIndexType: BasicIndex,
  };
  return createCollection(electricCollectionOptions(config));
}

function cachedByOrganization<T>(
  create: (organizationSlug: string, baseUrl: string | undefined) => T,
) {
  const cache = new Map<string, T>();
  return (organizationSlug: string, baseUrl?: string) => {
    const existing = cache.get(organizationSlug);
    if (existing) return existing;
    const collection = create(organizationSlug, baseUrl);
    cache.set(organizationSlug, collection);
    return collection;
  };
}

export const getProjectsCollection = cachedByCollectionScope(
  (organizationSlug, scope) => createApiCollection<ProjectRow>({
    queryClient: scope.queryClient,
    queryKey: ["collections", scope.sessionId, scope.userId, organizationSlug, "project"],
    queryFn: async ({ signal }) => {
      const rows = await readCollectionServerFn({ data: { table: "project", organizationSlug, userId: scope.userId }, signal });
      // SAFETY: the literal table selects project in the authenticated allowlisted read.
      return rows as ProjectRow[];
    },
    getKey: (row) => row.id,
  }),
);

export const getEnvironmentsCollection = cachedByCollectionScope(
  (organizationSlug, scope) => createApiCollection<EnvironmentRow>({
    queryClient: scope.queryClient,
    queryKey: ["collections", scope.sessionId, scope.userId, organizationSlug, "environment"],
    queryFn: async ({ signal }) => {
      const rows = await readCollectionServerFn({ data: { table: "environment", organizationSlug, userId: scope.userId }, signal });
      // SAFETY: the literal table selects environment in the authenticated allowlisted read.
      return rows as EnvironmentRow[];
    },
    getKey: (row) => row.id,
  }),
);

export const getRawServicesCollection = cachedByOrganization(
  (organizationSlug, baseUrl) =>
    makeOrganizationCollection<ServiceRow>({
      table: "service",
      organizationSlug,
      baseUrl,
      getKey: (row) => row.id,
    }),
);

export const getCanvasPositionsCollection = cachedByOrganization(
  (organizationSlug, baseUrl) =>
    makeOrganizationCollection<CanvasPositionRow>({
      table: "environment_canvas_node_position",
      organizationSlug,
      baseUrl,
      getKey: (row) => `${row.resourceType}:${row.resourceId}`,
    }),
);

export const getResourceLineagesCollection = cachedByOrganization(
  (organizationSlug, baseUrl) =>
    makeOrganizationCollection<ResourceLineageRow>({
      table: "resource_lineage",
      organizationSlug,
      baseUrl,
      getKey: (row) => row.id,
    }),
);

export const getRawEnvironmentResourcesCollection = cachedByOrganization(
  (organizationSlug, baseUrl) =>
    makeOrganizationCollection<EnvironmentResourceRow>({
      table: "environment_resource",
      organizationSlug,
      baseUrl,
      getKey: (row) => row.id,
    }),
);

export const getEnvironmentDeploymentsCollection = cachedByCollectionScope(
  (organizationSlug, scope) =>
    createApiCollection<EnvironmentDeploymentRow>({
      queryClient: scope.queryClient,
      queryKey: ["collections", scope.sessionId, scope.userId, organizationSlug, "environment_deployment"],
      queryFn: async ({ signal }) => {
        const rows = await readCollectionServerFn({ data: { table: "environment_deployment", organizationSlug, userId: scope.userId }, signal });
        // SAFETY: the literal table selects environment_deployment in the authenticated allowlisted read.
        return rows as EnvironmentDeploymentRow[];
      },
      getKey: (row) => row.id,
    }),
);

export const getEnvironmentSavedStateRevisionsCollection =
  cachedByCollectionScope((organizationSlug, scope) =>
    createApiCollection<EnvironmentSavedStateRevisionRow>({
      queryClient: scope.queryClient,
      queryKey: ["collections", scope.sessionId, scope.userId, organizationSlug, "environment_saved_state_snapshot"],
      queryFn: async ({ signal }) => {
        const rows = await readCollectionServerFn({ data: { table: "environment_saved_state_snapshot", organizationSlug, userId: scope.userId }, signal });
        // SAFETY: the literal table selects environment_saved_state_snapshot in the authenticated allowlisted read.
        return rows as EnvironmentSavedStateRevisionRow[];
      },
      getKey: (row) => row.id,
    }),
  );

export const getEnvironmentNodeConfigSnapshotsCollection =
  cachedByCollectionScope((organizationSlug, scope) =>
    createApiCollection<EnvironmentNodeConfigSnapshotRow>({
      queryClient: scope.queryClient,
      queryKey: ["collections", scope.sessionId, scope.userId, organizationSlug, "environment_node_config_snapshot"],
      queryFn: async ({ signal }) => {
        const rows = await readCollectionServerFn({ data: { table: "environment_node_config_snapshot", organizationSlug, userId: scope.userId }, signal });
        // SAFETY: the literal table selects environment_node_config_snapshot in the authenticated allowlisted read.
        return rows as EnvironmentNodeConfigSnapshotRow[];
      },
      getKey: (row) => row.id,
    }),
  );

export const getEnvironmentNodeIntroductionsCollection =
  cachedByCollectionScope((organizationSlug, scope) =>
    createApiCollection<EnvironmentNodeIntroductionRow>({
      queryClient: scope.queryClient,
      queryKey: ["collections", scope.sessionId, scope.userId, organizationSlug, "environment_node_introduction"],
      queryFn: async ({ signal }) => {
        const rows = await readCollectionServerFn({ data: { table: "environment_node_introduction", organizationSlug, userId: scope.userId }, signal });
        // SAFETY: the literal table selects environment_node_introduction in the authenticated allowlisted read.
        return rows as EnvironmentNodeIntroductionRow[];
      },
      getKey: (row) => `${row.nodeType}:${row.nodeId}`,
    }),
  );

export const getVolumeRemoveAttemptsCollection = cachedByCollectionScope(
  (organizationSlug, scope) =>
    createApiCollection<VolumeRemoveAttemptRow>({
      queryClient: scope.queryClient,
      queryKey: ["collections", scope.sessionId, scope.userId, organizationSlug, "volume_remove_attempt"],
      queryFn: async ({ signal }) => {
        const rows = await readCollectionServerFn({ data: { table: "volume_remove_attempt", organizationSlug, userId: scope.userId }, signal });
        // SAFETY: the literal table selects volume_remove_attempt in the authenticated allowlisted read.
        return rows as VolumeRemoveAttemptRow[];
      },
      getKey: (row) => row.id,
    }),
);
