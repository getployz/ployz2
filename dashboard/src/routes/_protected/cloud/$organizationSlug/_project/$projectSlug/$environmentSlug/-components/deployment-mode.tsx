import { createContext, use, useEffect, useRef, type ReactNode } from "react";
import { Link, retainSearchParams, useLoaderData, useParams, useSearch } from "@tanstack/react-router";
import { Schema } from "effect";
import { setOpenStartedDeployments } from "#/auth/open-started-deployments";
import { useCollectionScope } from "#/collections/use-collection-scope";
import { Button } from "#/components/ui/button";
import { ACTIVE_ENVIRONMENT_DEPLOYMENT_STATUSES } from "#/modules/deployments/runtime-contract";
import { useDeploymentAttempt, type DeploymentAttempt } from "#/modules/deployments/deployment.collection";
import { ENVIRONMENT_ROUTE_FROM } from "./environment-route-paths";

export const CANVAS_ROUTE_ID =
  "/_protected/cloud/$organizationSlug/_project/$projectSlug/$environmentSlug/_canvas";

/**
 * The canvas route's search: `deployment=<id>` puts the canvas into Deployment Mode for that
 * Cloud Deployment Attempt and is retained across navigations inside the canvas.
 * Removing it (Back to live, browser Back) returns to Live Mode.
 * `deploymentList=true` opens the deploy bar's deployment list; it is not retained.
 */
export const canvasRouteSearch = {
  validateSearch: Schema.toStandardSchemaV1(Schema.Struct({
    deployment: Schema.optional(Schema.String),
    deploymentList: Schema.optional(Schema.Boolean),
  })),
  search: { middlewares: [retainSearchParams<{ deployment?: string }>(["deployment"])] },
};

const DeploymentModeContext = createContext<DeploymentAttempt | null>(null);

/** The attempt the canvas shows in Deployment Mode, or null in Live Mode. Everything in Deployment Mode is read-only. */
export function useDeploymentMode() {
  return use(DeploymentModeContext);
}

// ponytail: an unknown or other-environment id falls back to Live Mode with the param still in the URL.
export function DeploymentModeProvider({ children }: { children: ReactNode }) {
  const { organizationSlug } = useParams({ from: ENVIRONMENT_ROUTE_FROM });
  const { environmentId } = useLoaderData({ from: ENVIRONMENT_ROUTE_FROM });
  const deploymentId = useSearch({ from: CANVAS_ROUTE_ID, select: (search) => search.deployment ?? null });
  const attempt = useDeploymentAttempt(organizationSlug, environmentId, deploymentId, { buildLog: true });
  const { userId } = useCollectionScope();
  const origin = attempt?.deployment.triggerOrigin;
  const ownRunningId = attempt && ACTIVE_ENVIRONMENT_DEPLOYMENT_STATUSES.has(attempt.deployment.status)
    && origin?.origin === "manual" && origin.actorId === userId ? attempt.deployment.id : null;
  // "Open deployments I start": opening your own running attempt turns it on; returning to live while it runs turns it off.
  const shownOwnRunning = useRef<string | null>(null);
  useEffect(() => {
    const previous = shownOwnRunning.current;
    shownOwnRunning.current = ownRunningId;
    if (ownRunningId !== null && ownRunningId !== previous) void setOpenStartedDeployments(true);
    else if (previous !== null && deploymentId === null) void setOpenStartedDeployments(false);
  }, [ownRunningId, deploymentId]);
  return <DeploymentModeContext value={attempt}>{children}</DeploymentModeContext>;
}

export function BackToLive({ className }: { className?: string }) {
  return (
    <Button
      size="sm"
      className={className}
      nativeButton={false}
      render={<Link to="." search={(previous) => ({ ...previous, deployment: undefined })} />}
    >
      Back to live
    </Button>
  );
}
