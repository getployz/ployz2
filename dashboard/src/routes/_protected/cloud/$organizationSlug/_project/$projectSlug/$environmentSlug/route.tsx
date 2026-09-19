import { DashboardShell } from "#/components/dashboard-shell";
import {
  environmentBySlugQueryOptions,
  rememberSelectedEnvironment,
  environmentListQueryOptions,
} from "#/modules/environment-design/workspace-queries";
import { projectBySlugQueryOptions } from "#/modules/environment-design/workspace-queries";
import {
  createFileRoute,
  notFound,
  Outlet,
  useParams,
} from "@tanstack/react-router";
import { hasPublicErrorCode } from "#/lib/public-error";

export const Route = createFileRoute(
  "/_protected/cloud/$organizationSlug/_project/$projectSlug/$environmentSlug",
)({
  loader: async ({ params, context }) => {
    const organizationSlug = params.organizationSlug;
    const navigationReady = Promise.all([
      context.queryClient.prefetchQuery(
        projectBySlugQueryOptions(organizationSlug, params.projectSlug),
      ),
      context.queryClient.prefetchQuery(
        environmentListQueryOptions(
          organizationSlug,
          params.projectSlug,
        ),
      ),
    ]);
    const environment = await context.queryClient
      .ensureQueryData(
        environmentBySlugQueryOptions(
          organizationSlug,
          params.projectSlug,
          params.environmentSlug,
        ),
      )
      .catch((cause: unknown) => {
        if (hasPublicErrorCode(cause, "NOT_FOUND")) throw notFound();
        throw cause;
      });

    return {
      environmentId: environment.id,
      organizationId: environment.organizationId,
      navigationReady,
    };
  },
  onEnter: ({ context, params }) => {
    void rememberSelectedEnvironment(context.queryClient, params);
  },
  onStay: ({ context, params }) => {
    void rememberSelectedEnvironment(context.queryClient, params);
  },
  component: RouteComponent,
});

function RouteComponent() {
  const params = useParams({
    from: "/_protected/cloud/$organizationSlug/_project/$projectSlug/$environmentSlug",
  });
  return <DashboardShell scope={{ kind: "environment", ...params }}><Outlet /></DashboardShell>;
}
