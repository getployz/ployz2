import { environmentManager, type FetchInfiniteQueryOptions, type FetchQueryOptions, type QueryClient, type QueryKey } from "@tanstack/react-query";
import { notFound } from "@tanstack/react-router";
import type { CollectionScope } from "./scope";
import { getEnvironmentDeploymentsCollection } from "./collections";
import { orgStoreOptions } from "./org-store";
import {
  loadWorkspaceEnvironment, organizationStateQueryOptions, preloadWorkspace, readWorkspace,
} from "#/modules/environment-design/workspace.queries";
import { deploymentBuildTailQueryOptions } from "#/modules/deployments/deployment-build-log.queries";
import { deploymentAttemptQueryOptions, environmentDeploymentsQueryOptions } from "#/modules/deployments/deployment-history.queries";
import type { EnvironmentBySlug } from "#/modules/environment-design/workspace-schemas";

/**
 * Route loaders call only the helpers in this file.
 * - `require*` awaits a route decision (access, not-found, redirect) on server and client.
 * - `prefetch*` awaits on the server so SSR HTML is complete, and never blocks client navigation.
 */
type RouteDataContext = {
  queryClient: QueryClient;
  session: { session: { id: string }; user: { id: string } };
};

function scopeOf(context: RouteDataContext): CollectionScope {
  return { queryClient: context.queryClient, sessionId: context.session.session.id, userId: context.session.user.id };
}

export async function requireOrganization(context: RouteDataContext, organizationSlug: string) {
  const organization = await context.queryClient.ensureQueryData(organizationStateQueryOptions(organizationSlug));
  if (organization.activeOrganization?.slug !== organizationSlug) throw notFound();
}

/** Billing exists only on Ployz-hosted Cloud. */
export async function requireBilling(context: RouteDataContext, organizationSlug: string) {
  const organization = await context.queryClient.ensureQueryData(organizationStateQueryOptions(organizationSlug));
  if (!organization.billingEnabled) throw notFound();
}

export async function requireWorkspace(context: RouteDataContext, organizationSlug: string) {
  return readWorkspace(await preloadWorkspace(organizationSlug, scopeOf(context)));
}

export async function requireEnvironment(context: RouteDataContext, input: EnvironmentBySlug) {
  return loadWorkspaceEnvironment(input, scopeOf(context));
}

/** SSR failure fails the organization route: no org page can render without the Org Store. */
export async function prefetchOrgStore(context: RouteDataContext, organizationSlug: string) {
  const ready = context.queryClient.ensureQueryData(orgStoreOptions(organizationSlug, scopeOf(context)));
  // The shell's content gate owns client pending and error state.
  if (environmentManager.isServer()) await ready;
  else void ready.catch(() => {});
}

/**
 * Starts every read together. Never throws: an SSR failure is retried by the page's
 * `useSuspenseQuery`, whose boundary owns the error.
 */
export async function prefetchRemote<T, K extends QueryKey>(context: RouteDataContext, ...reads: Array<FetchQueryOptions<T, Error, T, K>>) {
  const ready = Promise.all(reads.map((options) => context.queryClient.prefetchQuery(options)));
  if (environmentManager.isServer()) await ready;
}

/** `prefetchRemote` for a paged read: its first page. */
export async function prefetchRemotePages<T, K extends QueryKey, P>(context: RouteDataContext, options: FetchInfiniteQueryOptions<T, Error, T, K, P>) {
  const ready = context.queryClient.prefetchInfiniteQuery(options);
  if (environmentManager.isServer()) await ready;
}

/**
 * The canvas's Deployment Mode reads, all started together: the viewed attempt's build tail and, unless the Org Store holds
 * the attempt with its target node list (the canvas then draws from the Org Store alone), its per-attempt read; and the open
 * deployment list's first page, keyed by the environment id the Org Store resolves. The Org Store is in memory on client navigations.
 */
export async function prefetchDeploymentMode(context: RouteDataContext, params: EnvironmentBySlug, { deployment, deploymentList }: {
  deployment: string | null; deploymentList?: boolean;
}) {
  const { organizationSlug } = params;
  const scope = scopeOf(context);
  const inOrgStore = (deploymentId: string) => context.queryClient.ensureQueryData(orgStoreOptions(organizationSlug, scope)).then(
    () => getEnvironmentDeploymentsCollection(organizationSlug, scope).get(deploymentId)?.targetNodes != null,
    () => false,
  );
  await Promise.all([
    deployment && prefetchRemote(context, deploymentBuildTailQueryOptions(organizationSlug, deployment)),
    deployment && inOrgStore(deployment).then((held) => held ? undefined : prefetchRemote(context, deploymentAttemptQueryOptions(organizationSlug, deployment))),
    deploymentList && requireEnvironment(context, params).then((environment) =>
      prefetchRemotePages(context, environmentDeploymentsQueryOptions(organizationSlug, environment.id))),
  ]);
}
