import { infiniteQueryOptions, queryOptions, useSuspenseInfiniteQuery, useSuspenseQuery } from "@tanstack/react-query";
import { parseServiceConfig, type ServiceConfig } from "@ployz/sdk/config";
import type { CollectionScope } from "#/collections/scope";
import { getDeploymentAttemptServerFn, listEnvironmentDeploymentsServerFn, listNodeDeploymentsServerFn } from "./deployment.functions";

export type DeploymentHistoryRow = Awaited<ReturnType<typeof listEnvironmentDeploymentsServerFn>>["items"][number];
export type NodeDeployment = NonNullable<Awaited<ReturnType<typeof listNodeDeploymentsServerFn>>["running"]>;

const historyKey = (organizationSlug: string): Array<string | null> => ["deployment-history", organizationSlug];

/** Every deployment list pages the same way: newest first, each page naming the attempt the next one starts before. */
const pagedHistory = {
  // SAFETY: widens the first page's cursor (none) to the attempt id later pages start before.
  initialPageParam: undefined as string | undefined,
  getNextPageParam: (page: { next: string | null }) => page.next ?? undefined,
};

/** An environment's attempts, a page at a time. */
export function environmentDeploymentsQueryOptions(organizationSlug: string, environmentId: string) {
  return infiniteQueryOptions({
    ...pagedHistory,
    // Refetched when the change stream names a deployment row change (invalidateDeploymentHistory).
    staleTime: Infinity,
    queryKey: [...historyKey(organizationSlug), "environment", environmentId],
    queryFn: ({ pageParam, signal }) =>
      listEnvironmentDeploymentsServerFn({ data: { organizationSlug, environmentId, before: pageParam }, signal }),
  });
}

/** A node's History a page at a time; the first page also names its Running attempt. */
export function nodeDeploymentsQueryOptions(organizationSlug: string, environmentId: string, nodeId: string) {
  return infiniteQueryOptions({
    ...pagedHistory,
    // Refetched when the change stream names a deployment row change (invalidateDeploymentHistory).
    staleTime: Infinity,
    queryKey: [...historyKey(organizationSlug), "node", environmentId, nodeId],
    queryFn: ({ pageParam, signal }) =>
      listNodeDeploymentsServerFn({ data: { organizationSlug, environmentId, nodeId, before: pageParam }, signal }),
  });
}

export function useNodeDeployments(organizationSlug: string, environmentId: string, nodeId: string) {
  return useSuspenseInfiniteQuery(nodeDeploymentsQueryOptions(organizationSlug, environmentId, nodeId));
}

/**
 * One attempt's row and the service configs it deployed (plus, before target lists, its nodes from its snapshots);
 * null without an id, or when the organization has no such attempt.
 */
export function deploymentAttemptQueryOptions(organizationSlug: string, deploymentId: string | null) {
  return queryOptions({
    queryKey: [...historyKey(organizationSlug), "attempt", deploymentId],
    // A queued attempt's snapshots are rewritten with its row, which the change stream names (invalidateDeploymentHistory).
    staleTime: Infinity,
    initialData: () => deploymentId === null ? null : undefined,
    queryFn: ({ signal }) => deploymentId === null ? null : getDeploymentAttemptServerFn({ data: { organizationSlug, deploymentId }, signal }),
  });
}

/** A deployment row changed: its attempt read and every page that may hold it refetch. */
export function invalidateDeploymentHistory(organizationSlug: string, scope: CollectionScope) {
  void scope.queryClient.invalidateQueries({ queryKey: historyKey(organizationSlug) });
}

/**
 * The service configs one attempt deployed, by node id, for the service panel's Details. Suspends until they arrive.
 * A removed service has none: the attempt holds no snapshot of it.
 */
export function useAttemptServiceConfigs(organizationSlug: string, deploymentId: string): Map<string, ServiceConfig> {
  const { data } = useSuspenseQuery(deploymentAttemptQueryOptions(organizationSlug, deploymentId));
  return new Map(data?.serviceConfigs.map((row) => [row.nodeId, parseServiceConfig(row.config)]));
}
