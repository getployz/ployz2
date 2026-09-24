import type { QueryCollectionUtils } from "@tanstack/query-db-collection";
import type { ChangeName } from "./change-sources";
import type { CollectionRead, CollectionReadInput } from "./read.contract";
import type { OrganizationEnrollmentRow } from "#/modules/machines/enrollment";
import { createChangeCollection } from "#/collections/query-collection";
import { readCollectionServerFn } from "#/collections/read.functions";
import { cachedByCollectionScope, type CollectionScope } from "#/collections/scope";
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

/** Every Org Store collection is fed by the Organization change log: a refetch reads only rows changed `since` its cursor. */
function collectionChangeOptions<Row>(table: CollectionReadInput["table"], organizationSlug: string, scope: CollectionScope) {
  return {
    queryClient: scope.queryClient,
    queryKey: ["collections", scope.sessionId, scope.userId, organizationSlug, table],
    read: async ({ signal, since }: { signal: AbortSignal; since: string | undefined }) => {
      // SAFETY: each owner below pairs its literal allowlisted table with that table's database row type.
      return await readCollectionServerFn({ data: { table, organizationSlug, userId: scope.userId, since }, signal }) as CollectionRead<Row>;
    },
  };
}

export const getProjectsCollection = cachedByCollectionScope(
  (organizationSlug, scope) => createChangeCollection<ProjectRow>({
    ...collectionChangeOptions<ProjectRow>("project", organizationSlug, scope),
    getKey: (row) => row.id,
  }),
);

export const getEnvironmentsCollection = cachedByCollectionScope(
  (organizationSlug, scope) => createChangeCollection<EnvironmentRow>({
    ...collectionChangeOptions<EnvironmentRow>("environment", organizationSlug, scope),
    getKey: (row) => row.id,
  }),
);

export const getRawServicesCollection = cachedByCollectionScope(
  (organizationSlug, scope) => createChangeCollection<ServiceRow>({
    ...collectionChangeOptions<ServiceRow>("service", organizationSlug, scope),
    getKey: (row) => row.id,
  }),
);

export const getCanvasPositionsCollection = cachedByCollectionScope(
  (organizationSlug, scope) => createChangeCollection<CanvasPositionRow>({
    ...collectionChangeOptions<CanvasPositionRow>("environment_canvas_node_position", organizationSlug, scope),
    getKey: (row) => `${row.resourceType}:${row.resourceId}`,
  }),
);

export const getResourceLineagesCollection = cachedByCollectionScope(
  (organizationSlug, scope) => createChangeCollection<ResourceLineageRow>({
    ...collectionChangeOptions<ResourceLineageRow>("resource_lineage", organizationSlug, scope),
    getKey: (row) => row.id,
  }),
);

export const getRawEnvironmentResourcesCollection = cachedByCollectionScope(
  (organizationSlug, scope) => createChangeCollection<EnvironmentResourceRow>({
    ...collectionChangeOptions<EnvironmentResourceRow>("environment_resource", organizationSlug, scope),
    getKey: (row) => row.id,
  }),
);

export const getEnvironmentDeploymentsCollection = cachedByCollectionScope(
  (organizationSlug, scope) =>
    createChangeCollection<EnvironmentDeploymentRow>({
    ...collectionChangeOptions<EnvironmentDeploymentRow>("environment_deployment", organizationSlug, scope),
    getKey: (row) => row.id,
    }),
);

export const getEnvironmentSavedStateRevisionsCollection =
  cachedByCollectionScope((organizationSlug, scope) =>
    createChangeCollection<EnvironmentSavedStateRevisionRow>({
    ...collectionChangeOptions<EnvironmentSavedStateRevisionRow>("environment_saved_state_snapshot", organizationSlug, scope),
    getKey: (row) => row.id,
    }),
  );

export const getEnvironmentNodeConfigSnapshotsCollection =
  cachedByCollectionScope((organizationSlug, scope) =>
    createChangeCollection<EnvironmentNodeConfigSnapshotRow>({
    ...collectionChangeOptions<EnvironmentNodeConfigSnapshotRow>("environment_node_config_snapshot", organizationSlug, scope),
    getKey: (row) => row.id,
    }),
  );

export const getEnvironmentNodeIntroductionsCollection =
  cachedByCollectionScope((organizationSlug, scope) =>
    createChangeCollection<EnvironmentNodeIntroductionRow>({
    ...collectionChangeOptions<EnvironmentNodeIntroductionRow>("environment_node_introduction", organizationSlug, scope),
    getKey: (row) => `${row.nodeType}:${row.nodeId}`,
    }),
  );

export const getVolumeRemoveAttemptsCollection = cachedByCollectionScope(
  (organizationSlug, scope) =>
    createChangeCollection<VolumeRemoveAttemptRow>({
    ...collectionChangeOptions<VolumeRemoveAttemptRow>("volume_remove_attempt", organizationSlug, scope),
    getKey: (row) => row.id,
    }),
);

export const getOrganizationEnrollmentCollection = cachedByCollectionScope((organizationSlug, scope) =>
  createChangeCollection<OrganizationEnrollmentRow>({
    ...collectionChangeOptions<OrganizationEnrollmentRow>("organization_enrollment", organizationSlug, scope),
    getKey: (row) => row.id,
  }));

export type EnvironmentSummary = Pick<EnvironmentRow, "id" | "projectId" | "organizationId" | "name" | "namespace" | "createdAt">;
export function environmentSummary(row: EnvironmentSummary): EnvironmentSummary {
  const { id, projectId, organizationId, name, namespace, createdAt } = row;
  return { id, projectId, organizationId, name, namespace, createdAt };
}
export type ProjectPreference = { id: string; environmentId: string };

export const getEnvironmentSummariesCollection = cachedByCollectionScope((organizationSlug, scope) =>
  createChangeCollection<EnvironmentSummary>({
    ...collectionChangeOptions<EnvironmentSummary>("environment_summary", organizationSlug, scope),
    getKey: row => row.id,
  }));

export const getProjectPreferencesCollection = cachedByCollectionScope((organizationSlug, scope) =>
  createChangeCollection<ProjectPreference>({
    ...collectionChangeOptions<ProjectPreference>("project_preference", organizationSlug, scope),
    getKey: row => row.id,
  }));

/**
 * Every Org Store collection by the name the Organization change stream sends.
 * Not a `get*Collection` export, so the Org Store gate doesn't count it twice.
 */
export const changeCollections = {
  project: getProjectsCollection,
  environment: getEnvironmentsCollection,
  environment_summary: getEnvironmentSummariesCollection,
  project_preference: getProjectPreferencesCollection,
  service: getRawServicesCollection,
  resource_lineage: getResourceLineagesCollection,
  environment_resource: getRawEnvironmentResourcesCollection,
  environment_canvas_node_position: getCanvasPositionsCollection,
  environment_deployment: getEnvironmentDeploymentsCollection,
  environment_saved_state_snapshot: getEnvironmentSavedStateRevisionsCollection,
  environment_node_config_snapshot: getEnvironmentNodeConfigSnapshotsCollection,
  environment_node_introduction: getEnvironmentNodeIntroductionsCollection,
  volume_remove_attempt: getVolumeRemoveAttemptsCollection,
  organization_enrollment: getOrganizationEnrollmentCollection,
} satisfies Record<Exclude<ChangeName, "organization">, (organizationSlug: string, scope: CollectionScope) => {
  utils: Pick<QueryCollectionUtils, "refetch">;
}>;
