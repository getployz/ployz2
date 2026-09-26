import { infiniteQueryOptions, useSuspenseInfiniteQuery } from "@tanstack/react-query";
import type { CollectionScope } from "#/collections/scope";
import { listNodeDeploymentsServerFn } from "./deployment.functions";

export type NodeDeployment = NonNullable<Awaited<ReturnType<typeof listNodeDeploymentsServerFn>>["running"]>;

const organizationKey = (organizationSlug: string) => ["node-deployments", organizationSlug] as const;

/** A node's Running attempt and its History, 20 attempts a page. */
export function nodeDeploymentsQueryOptions(organizationSlug: string, environmentId: string, nodeId: string) {
  return infiniteQueryOptions({
    queryKey: [...organizationKey(organizationSlug), environmentId, nodeId],
    // The change stream refetches it when deployment rows change.
    staleTime: Infinity,
    // SAFETY: widens the first page's cursor (none) to the attempt id later pages start before.
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) => listNodeDeploymentsServerFn({ data: { organizationSlug, environmentId, nodeId, before: pageParam }, signal }),
    getNextPageParam: (page) => page.next ?? undefined,
  });
}

export function useNodeDeployments(organizationSlug: string, environmentId: string, nodeId: string) {
  return useSuspenseInfiniteQuery(nodeDeploymentsQueryOptions(organizationSlug, environmentId, nodeId));
}

export function refetchNodeDeployments(organizationSlug: string, scope: CollectionScope) {
  void scope.queryClient.invalidateQueries({ queryKey: organizationKey(organizationSlug) });
}
