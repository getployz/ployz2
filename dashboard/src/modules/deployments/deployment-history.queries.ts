import { infiniteQueryOptions, queryOptions, useSuspenseQuery, type QueryClient } from "@tanstack/react-query";
import { parseServiceConfig, type ServiceConfig } from "@ployz/sdk/config";
import type { CollectionScope } from "#/collections/scope";
import { getDeploymentAttemptServerFn, listEnvironmentDeploymentsServerFn } from "./deployment.functions";

type DeploymentPage = Awaited<ReturnType<typeof listEnvironmentDeploymentsServerFn>>;
export type DeploymentHistoryRow = DeploymentPage["rows"][number];

const historyKey = (organizationSlug: string): Array<string | null> => ["deployment-history", organizationSlug];

/** An environment's attempts, a page of 20 at a time, newest first. */
export function environmentDeploymentsQueryOptions(organizationSlug: string, environmentId: string) {
  return infiniteQueryOptions({
    queryKey: [...historyKey(organizationSlug), "environment", environmentId],
    // Refetched when the change stream names a deployment row change (invalidateDeploymentHistory).
    staleTime: Infinity,
    // SAFETY: widens the first page's cursor (none) to the attempt id later pages start before.
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page: DeploymentPage) => page.next ?? undefined,
    queryFn: ({ pageParam, signal }) =>
      listEnvironmentDeploymentsServerFn({ data: { organizationSlug, environmentId, before: pageParam }, signal }),
  });
}

/** Starts the list's first page before it opens. */
export function warmEnvironmentDeployments(queryClient: QueryClient, organizationSlug: string, environmentId: string) {
  void queryClient.prefetchInfiniteQuery(environmentDeploymentsQueryOptions(organizationSlug, environmentId));
}

/** One attempt's row and the service configs it deployed; null without an id, or when the organization has no such attempt. */
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
 * The service configs one attempt deployed, by node id, for card details (icon, source, mounts, the panel's Details).
 * A removed service has none: the attempt holds no snapshot of it.
 */
export function useAttemptServiceConfigs(organizationSlug: string, deploymentId: string): Map<string, ServiceConfig> {
  const { data } = useSuspenseQuery(deploymentAttemptQueryOptions(organizationSlug, deploymentId));
  return new Map(data?.serviceConfigs.map((row) => [row.nodeId, parseServiceConfig(row.config)]));
}
