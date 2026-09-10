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

export const getRawServicesCollection = cachedByCollectionScope(
  (organizationSlug, scope) => createApiCollection<ServiceRow>({
    queryClient: scope.queryClient,
    queryKey: ["collections", scope.sessionId, scope.userId, organizationSlug, "service"],
    queryFn: async ({ signal }) => {
      const rows = await readCollectionServerFn({ data: { table: "service", organizationSlug, userId: scope.userId }, signal });
      // SAFETY: the literal table selects service in the authenticated allowlisted read.
      return rows as ServiceRow[];
    },
    getKey: (row) => row.id,
  }),
);

export const getCanvasPositionsCollection = cachedByCollectionScope(
  (organizationSlug, scope) => createApiCollection<CanvasPositionRow>({
    queryClient: scope.queryClient,
    queryKey: ["collections", scope.sessionId, scope.userId, organizationSlug, "environment_canvas_node_position"],
    queryFn: async ({ signal }) => {
      const rows = await readCollectionServerFn({ data: { table: "environment_canvas_node_position", organizationSlug, userId: scope.userId }, signal });
      // SAFETY: the literal table selects environment_canvas_node_position in the authenticated allowlisted read.
      return rows as CanvasPositionRow[];
    },
    getKey: (row) => `${row.resourceType}:${row.resourceId}`,
  }),
);

export const getResourceLineagesCollection = cachedByCollectionScope(
  (organizationSlug, scope) => createApiCollection<ResourceLineageRow>({
    queryClient: scope.queryClient,
    queryKey: ["collections", scope.sessionId, scope.userId, organizationSlug, "resource_lineage"],
    queryFn: async ({ signal }) => {
      const rows = await readCollectionServerFn({ data: { table: "resource_lineage", organizationSlug, userId: scope.userId }, signal });
      // SAFETY: the literal table selects resource_lineage in the authenticated allowlisted read.
      return rows as ResourceLineageRow[];
    },
    getKey: (row) => row.id,
  }),
);

export const getRawEnvironmentResourcesCollection = cachedByCollectionScope(
  (organizationSlug, scope) => createApiCollection<EnvironmentResourceRow>({
    queryClient: scope.queryClient,
    queryKey: ["collections", scope.sessionId, scope.userId, organizationSlug, "environment_resource"],
    queryFn: async ({ signal }) => {
      const rows = await readCollectionServerFn({ data: { table: "environment_resource", organizationSlug, userId: scope.userId }, signal });
      // SAFETY: the literal table selects environment_resource in the authenticated allowlisted read.
      return rows as EnvironmentResourceRow[];
    },
    getKey: (row) => row.id,
  }),
);

export const getEnvironmentDeploymentsCollection = cachedByOrganization(
  (organizationSlug, baseUrl) =>
    makeOrganizationCollection<EnvironmentDeploymentRow>({
      table: "environment_deployment",
      organizationSlug,
      baseUrl,
      getKey: (row) => row.id,
    }),
);

export const getEnvironmentSavedStateRevisionsCollection =
  cachedByOrganization((organizationSlug, baseUrl) =>
    makeOrganizationCollection<EnvironmentSavedStateRevisionRow>({
      table: "environment_saved_state_snapshot",
      organizationSlug,
      baseUrl,
      getKey: (row) => row.id,
    }),
  );

export const getEnvironmentNodeConfigSnapshotsCollection =
  cachedByOrganization((organizationSlug, baseUrl) =>
    makeOrganizationCollection<EnvironmentNodeConfigSnapshotRow>({
      table: "environment_node_config_snapshot",
      organizationSlug,
      baseUrl,
      getKey: (row) => row.id,
    }),
  );

export const getEnvironmentNodeIntroductionsCollection =
  cachedByOrganization((organizationSlug, baseUrl) =>
    makeOrganizationCollection<EnvironmentNodeIntroductionRow>({
      table: "environment_node_introduction",
      organizationSlug,
      baseUrl,
      getKey: (row) => `${row.nodeType}:${row.nodeId}`,
    }),
  );

export const getVolumeRemoveAttemptsCollection = cachedByOrganization(
  (organizationSlug, baseUrl) =>
    makeOrganizationCollection<VolumeRemoveAttemptRow>({
      table: "volume_remove_attempt",
      organizationSlug,
      baseUrl,
      getKey: (row) => row.id,
    }),
);
