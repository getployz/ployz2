import { reconcileCollection } from "#/collections/query-collection";
import { cachedByCollectionScope, getDbClient, type CollectionScope } from "#/collections/scope";
import {
  and, collectionOptions, liveQueryCollectionOptions,
  eq,
  useLiveSuspenseQuery,
} from "@tanstack/react-db";
import { useCollectionScope } from "#/collections/use-collection-scope";
import { attemptNodes, deploymentView, type AttemptTargetNode, type BuildLog, type DeploymentView } from "#/modules/deployments/deployment-view";
import {
  getEnvironmentDeploymentsCollection,
  getEnvironmentNodeConfigSnapshotsCollection,
  getEnvironmentsCollection,
  getProjectsCollection,
} from "#/collections/collections";
import { decodeStrict } from "#/modules/environment-design/schema";
import {
  environmentDeploymentSummarySchema,
  type EnvironmentDeploymentSummary,
} from "#/modules/deployments/deployment-contract";
import { parseServiceConfig, type ServiceConfig } from "@ployz/sdk/config";
import { parseSdkDeployPreview } from "#/modules/deployments/runtime-preview";
import { useBuildTail } from "#/modules/deployments/deployment-build-log.queries";

export const getOrganizationDeploymentsCollection = cachedByCollectionScope((organizationSlug, scope) => {
  const client = getDbClient(scope.queryClient);
  const deployments = getEnvironmentDeploymentsCollection(organizationSlug, scope);
  const environments = getEnvironmentsCollection(organizationSlug, scope);
  const projects = getProjectsCollection(organizationSlug, scope);

  const rows = client.collection(collectionOptions(liveQueryCollectionOptions({
    id: `${deployments.id}:deployment-relationships`,
    query: (q) => q
      .from({ deployment: deployments })
      .innerJoin({ environment: environments }, ({ deployment, environment }) =>
        eq(deployment.environmentId, environment.id),
      )
      .innerJoin({ project: projects }, ({ environment, project }) =>
        eq(environment.projectId, project.id),
      )
      .select(({ deployment, environment, project }) => ({
        deployment,
        projectSlug: project.slug,
        environmentSlug: environment.namespace,
      })),
  })));

  const collection =
    client.collection(collectionOptions(liveQueryCollectionOptions({
        id: `${deployments.id}:deployment-summaries`,
    query: (q) =>
      q.from({ deploymentRelationships: rows }).fn.select(({ deploymentRelationships }) => {
        const deployment = deploymentRelationships.deployment;
        const decoded = decodeStrict(
          environmentDeploymentSummarySchema,
          {
            id: deployment.id,
            environmentId: deployment.environmentId,
            triggerOrigin: deployment.triggerOrigin,
            status: deployment.status,
            message: deployment.message,
            failureMessage: deployment.failureMessage,
            inngestRunId: deployment.inngestRunId,
            coreDeployId: deployment.coreDeployId,
            deployPreview: deployment.deployPreview,
            runtimeProgress: deployment.runtimeProgress,
            sourcePins: deployment.sourcePins,
            targetNodes: deployment.targetNodes,
            buildServiceIds: deployment.targetNodes?.nodes.filter((node) => node.needsBuild).map((node) => node.nodeId) ?? [],
            canRetry: deployment.canRetry,
            failureCode: deployment.failureCode,
            dispatchRequestedAt: deployment.dispatchRequestedAt,
            startedAt: deployment.startedAt,
            finishedAt: deployment.finishedAt,
            cancellationRequestedAt: deployment.cancellationRequestedAt,
            createdAt: deployment.createdAt,
            updatedAt: deployment.updatedAt,
            projectSlug: deploymentRelationships.projectSlug,
            environmentSlug: deploymentRelationships.environmentSlug,
          },
        );
        return {
          ...decoded,
          deployPreview:
            deployment.deployPreview === null
              ? null
              : parseSdkDeployPreview(deployment.deployPreview),
        } satisfies EnvironmentDeploymentSummary;
      }),
        getKey: (item) => item.id,
      })));

  return collection;
});

/** Admission and Saved State commands can also replace a queued attempt's history. */
export async function reconcileDeploymentCollections(organizationSlug: string, scope: CollectionScope) {
  await Promise.all([
    reconcileCollection(getEnvironmentDeploymentsCollection(organizationSlug, scope)),
    reconcileCollection(getEnvironmentNodeConfigSnapshotsCollection(organizationSlug, scope)),
  ]);
}

/** `buildPending`: the build tail is still on its way, so build nodes' stages are unknown yet. */
export type DeploymentAttempt = { deployment: EnvironmentDeploymentSummary; nodes: AttemptTargetNode[]; view: DeploymentView; buildPending: boolean };

/** An environment's attempts newest first, each projected from its frozen target list. */
function useEnvironmentAttemptInputs(organizationSlug: string, environmentId: string) {
  const summaries = getOrganizationDeploymentsCollection(organizationSlug, useCollectionScope());
  const { data: attempts } = useLiveSuspenseQuery({
    queryKey: ["environment-deployment-attempts", summaries.id, environmentId],
    query: (q) => q.from({ deployment: summaries }).where(({ deployment }) => eq(deployment.environmentId, environmentId))
      .orderBy(({ deployment }) => deployment.createdAt, "desc"),
  });
  const project = (deployment: EnvironmentDeploymentSummary, buildLog?: BuildLog | null): DeploymentAttempt => {
    const { nodes, progress } = attemptNodes(deployment.targetNodes, deployment.runtimeProgress);
    return { deployment, nodes, view: deploymentView({ deployment, progress, nodes, buildLog }), buildPending: false };
  };
  return { attempts, project };
}

/**
 * One Cloud Deployment Attempt of an environment through the deployment view projection; null when the environment has no such attempt.
 * `buildLog` also reads the attempt's Build Steps and output tails (polled until it finishes) for per-image build stages and tails.
 */
export function useDeploymentAttempt(organizationSlug: string, environmentId: string, deploymentId: string | null, { buildLog = false } = {}): DeploymentAttempt | null {
  const { attempts, project } = useEnvironmentAttemptInputs(organizationSlug, environmentId);
  const deployment = attempts.find((candidate) => candidate.id === deploymentId);
  const tailId = buildLog && deployment?.buildServiceIds.length ? deployment.id : null;
  const tail = useBuildTail(organizationSlug, tailId);
  return deployment ? { ...project(deployment, tail.data), buildPending: tailId !== null && tail.isPending } : null;
}

/** Every Cloud Deployment Attempt of an environment through the deployment view projection, newest first. */
// ponytail: projects every attempt on each change; page the history if environments grow long ones.
export function useEnvironmentDeployments(organizationSlug: string, environmentId: string): DeploymentAttempt[] {
  const { attempts, project } = useEnvironmentAttemptInputs(organizationSlug, environmentId);
  return attempts.map((deployment) => project(deployment));
}

/** The environment's attempts whose target holds this node, newest first, each with the node's view. */
export function useNodeDeployments(organizationSlug: string, environmentId: string, nodeId: string) {
  return useEnvironmentDeployments(organizationSlug, environmentId).flatMap(({ deployment, view }) => {
    const node = view.nodes.find((candidate) => candidate.nodeId === nodeId);
    return node ? [{ deployment, node }] : [];
  });
}

/**
 * The service configs one attempt deployed, by node id, for card details (icon, source, mounts, the panel's Details).
 * A removed service has none: the attempt holds no snapshot of it.
 */
// ponytail: reads the node config snapshots collection for this one attempt; the per-attempt Remote Read replaces it.
export function useAttemptServiceConfigs(organizationSlug: string, deploymentId: string): Map<string, ServiceConfig> {
  const snapshots = getEnvironmentNodeConfigSnapshotsCollection(organizationSlug, useCollectionScope());
  const { data } = useLiveSuspenseQuery({
    queryKey: ["deployment-attempt-service-configs", snapshots.id, deploymentId],
    query: (q) => q.from({ snapshot: snapshots })
      .where(({ snapshot }) => and(eq(snapshot.environmentDeploymentId, deploymentId), eq(snapshot.nodeType, "service")))
      .select(({ snapshot }) => ({ nodeId: snapshot.nodeId, config: snapshot.config })),
  });
  return new Map(data.map((row) => [row.nodeId, parseServiceConfig(row.config)]));
}
