import { useQuery } from "@tanstack/react-query";
import { useParams } from "@tanstack/react-router";
import {
  getDashboardSectionLabel,
  type DashboardScope,
} from "./dashboard-navigation-model";
import { useDashboardSection } from "./use-dashboard-section";
import { environmentBySlugQueryOptions } from "#/modules/environment-design/workspace-queries";

function EnvironmentName({
  scope,
}: {
  scope: Extract<DashboardScope, { kind: "environment" }>;
}) {
  const { data } = useQuery(
    environmentBySlugQueryOptions(
      scope.organizationSlug,
      scope.projectSlug,
      scope.environmentSlug,
    ),
  );
  return (
    <span className="truncate text-muted-foreground">
      {data?.name ?? scope.environmentSlug}
    </span>
  );
}

export function DashboardPageHeader() {
  const { organizationSlug, projectSlug, environmentSlug } = useParams({
    strict: false,
  });
  const section = useDashboardSection();
  if (!organizationSlug) return null;
  const scope: DashboardScope =
    projectSlug && environmentSlug
      ? { kind: "environment", organizationSlug, projectSlug, environmentSlug }
      : { kind: "all", organizationSlug };
  return (
    <header
      data-dashboard-header
      className="hidden h-16 shrink-0 items-center gap-4 border-b bg-background px-6 min-wf-nav:flex"
    >
      <h1 className="truncate font-semibold">{getDashboardSectionLabel(scope, section)}</h1>
      {scope.kind === "environment" ? <EnvironmentName scope={scope} /> : null}
    </header>
  );
}
