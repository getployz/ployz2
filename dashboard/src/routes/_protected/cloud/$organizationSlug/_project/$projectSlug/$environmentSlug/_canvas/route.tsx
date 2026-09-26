import { Suspense } from "react";
import {
  createFileRoute,
  type ErrorComponentProps,
} from "@tanstack/react-router";
import { prefetchRemote, prefetchRemotePages, requireEnvironment } from "#/collections/route-data";
import { RouteErrorAlert } from "#/components/route-error-alert";
import { deploymentBuildTailQueryOptions } from "#/modules/deployments/deployment-build-log.queries";
import { deploymentAttemptQueryOptions, environmentDeploymentsQueryOptions } from "#/modules/deployments/deployment-history.queries";
import {
  EnvironmentCanvasScene,
  PendingCanvas,
} from "#/routes/_protected/cloud/$organizationSlug/_project/$projectSlug/$environmentSlug/-components/EnvironmentCanvasScene";
import { canvasRouteSearch } from "#/routes/_protected/cloud/$organizationSlug/_project/$projectSlug/$environmentSlug/-components/deployment-mode";

export const Route = createFileRoute(
  "/_protected/cloud/$organizationSlug/_project/$projectSlug/$environmentSlug/_canvas",
)({
  ...canvasRouteSearch,
  loaderDeps: ({ search }) => ({ deployment: search.deployment, deploymentList: search.deploymentList }),
  // Deployment Mode's nodes read the attempt and its build tail, and the open list its first page;
  // SSR renders them and hover preload warms them.
  loader: async ({ params, context, deps }) => {
    if (deps.deployment) {
      await prefetchRemote(context,
        deploymentBuildTailQueryOptions(params.organizationSlug, deps.deployment), deploymentAttemptQueryOptions(params.organizationSlug, deps.deployment));
    }
    if (deps.deploymentList) {
      const environment = await requireEnvironment(context, params);
      await prefetchRemotePages(context, environmentDeploymentsQueryOptions(params.organizationSlug, environment.id));
    }
  },
  errorComponent: CanvasError,
  component: CanvasLayout,
});

function CanvasLayout() {
  return (
    <div className="h-full overflow-hidden">
      <Suspense fallback={<PendingCanvas />}>
        <EnvironmentCanvasScene />
      </Suspense>
    </div>
  );
}

function CanvasError({ error }: ErrorComponentProps) {
  return (
    <div className="flex h-full items-center justify-center p-6">
      <RouteErrorAlert
        title="Couldn’t load this environment"
        description={
          error.message || "The environment data could not be loaded."
        }
      />
    </div>
  );
}
