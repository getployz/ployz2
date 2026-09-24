import { Suspense } from "react";
import { useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useServerFn } from "@tanstack/react-start";
import { prefetchRemote } from "#/collections/route-data";
import { DashboardPage } from "#/components/dashboard-page";
import { Skeleton } from "#/components/ui/skeleton";
import { resetPendingOrganizationEnrollmentServerFn } from "#/modules/machines/enrollment.functions";
import { organizationEnrollmentStatusQueryOptions } from "#/modules/machines/enrollment.queries";
import { TeardownDangerSection } from "#/routes/_protected/cloud/$organizationSlug/-components/teardown-danger-section";
import { PendingEnrollmentResetSection } from "#/routes/_protected/cloud/$organizationSlug/_org/-components/PendingEnrollmentResetSection";

export const Route = createFileRoute(
  "/_protected/cloud/$organizationSlug/_org/~/settings",
)({
  loader: async ({ params, context }) => {
    await prefetchRemote(context, organizationEnrollmentStatusQueryOptions(params.organizationSlug));
  },
  component: RouteComponent,
});

function RouteComponent() {
  const { organizationSlug } = Route.useParams();
  const navigate = useNavigate();

  return (
    <DashboardPage width="content">
      <h1 className="sr-only">Server Settings</h1>
      <Suspense fallback={<Skeleton className="h-24 w-full" />}>
        <EnrollmentSection organizationSlug={organizationSlug} />
      </Suspense>
      <TeardownDangerSection
        organizationSlug={organizationSlug}
        scope="organization"
        confirmPhrase={organizationSlug}
        title="Tear down this organization"
        description="Deletes all projects and their stored data, removes servers from the cluster, disconnects the cluster from Ployz, and deletes this organization. This cannot be undone."
        actionLabel="Tear down organization"
        headingId="organization-teardown-heading"
        onCompleted={() => {
          void navigate({ to: "/cloud", replace: true });
        }}
      />
    </DashboardPage>
  );
}

function EnrollmentSection({ organizationSlug }: { organizationSlug: string }) {
  const queryClient = useQueryClient();
  const options = organizationEnrollmentStatusQueryOptions(organizationSlug);
  const { data: status } = useSuspenseQuery(options);
  const resetPendingEnrollment = useServerFn(resetPendingOrganizationEnrollmentServerFn);
  return (
    <PendingEnrollmentResetSection
      status={status}
      onReset={({ confirmedFounderStoppedOrErased }) =>
        resetPendingEnrollment({
          data: { organizationSlug, confirmedFounderStoppedOrErased },
        }).then(() => undefined)
      }
      onCompleted={() => {
        void queryClient.invalidateQueries({ queryKey: options.queryKey });
      }}
    />
  );
}
