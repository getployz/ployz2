import { useState } from "react";
import { Link, useParams, useRouter } from "@tanstack/react-router";
import { useLiveQuery } from "@tanstack/react-db";
import { MoreVerticalIcon, XIcon } from "lucide-react";
import { toast } from "sonner";
import { CancelDeploymentDialog } from "#/components/cancel-deployment-dialog";
import { reconcileDeploymentCollections, useDeploymentAttempt } from "#/modules/deployments/deployment.collection";
import { deploymentStatusLabel } from "#/modules/deployments/deployment-view";
import { ENVIRONMENT_INDEX_ROUTE_TO } from "#/routes/_protected/cloud/$organizationSlug/_project/$projectSlug/$environmentSlug/-components/environment-route-paths";
import { useCollectionScope } from "#/collections/use-collection-scope";
import { formatRelativeTime } from "#/utils/relative-time";
import { VolumeRemoveAttemptHistory } from "#/components/volume-remove/deployment-volume-remove-history";
import { Button } from "#/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuTrigger } from "#/components/ui/dropdown-menu";
import { getRawEnvironmentResourcesCollection } from "#/collections/collections";
import type { EnvironmentDeploymentSummary } from "#/modules/deployments/deployment-contract";
import { dispatchQueuedEnvironmentDeploymentServerFn, retryEnvironmentDeploymentServerFn } from "#/modules/deployments/deployment.functions";

/** Retry re-admits a failed attempt's frozen target. */
export function useRetryDeployment(deployment: EnvironmentDeploymentSummary) {
  const [isRetrying, setIsRetrying] = useState(false);
  const { organizationSlug } = useParams({ strict: false });
  const collectionScope = useCollectionScope();
  async function retryDeployment() {
    if (!organizationSlug || !deployment.canRetry) return;
    setIsRetrying(true);
    try {
      await retryEnvironmentDeploymentServerFn({
          data: {
            organizationSlug,
            projectSlug: deployment.projectSlug,
            environmentSlug: deployment.environmentSlug,
            failedDeploymentId: deployment.id,
          },
        });
      await reconcileDeploymentCollections(organizationSlug, collectionScope);
      toast.success("Deployment retry queued.");
    } catch {
      toast.error("Could not retry this deployment.");
    } finally {
      setIsRetrying(false);
    }
  }
  return { retryDeployment, isRetrying };
}

export function DeploymentRow({ deployment }: { deployment: EnvironmentDeploymentSummary }) {
  const { retryDeployment, isRetrying } = useRetryDeployment(deployment);
  const [isDispatching, setIsDispatching] = useState(false);
  const router = useRouter();
  const { organizationSlug } = useParams({ strict: false });
  const collectionScope = useCollectionScope();
  const attempt = useDeploymentAttempt(organizationSlug ?? "", deployment.environmentId, deployment.id);
  const rawResources = getRawEnvironmentResourcesCollection(
    organizationSlug ?? "", collectionScope,
  );
  const { data: resourceRows = [] } = useLiveQuery({
    queryKey: ['deployment-resource-types', rawResources.id],
    query: (q) =>
      q
        .from({ resource: rawResources })
        .select(({ resource }) => ({
          id: resource.id,
          implementationType: resource.implementationType,
        })),
  });
  const availableVolumeResourceIds = new Set(
    resourceRows.flatMap((resource) =>
      resource.implementationType === "volume" ? [resource.id] : [],
    ),
  );
  const queuedForNextTrigger = deployment.status === "queued" && !deployment.dispatchRequestedAt;
  async function deployQueuedTarget() {
    if (!organizationSlug || !queuedForNextTrigger) return;
    setIsDispatching(true);
    try {
      await dispatchQueuedEnvironmentDeploymentServerFn({
          data: {
            organizationSlug,
            projectSlug: deployment.projectSlug,
            environmentSlug: deployment.environmentSlug,
          },
        });
      await reconcileDeploymentCollections(organizationSlug, collectionScope);
      await router.invalidate();
      toast.success("Deployment requested.");
    } catch {
      toast.error("Could not request this deployment.");
    } finally {
      setIsDispatching(false);
    }
  }

  const [cancelOpen, setCancelOpen] = useState(false);

  if (!attempt) return null;

  const cancelling = ["queued", "planning", "deploying"].includes(deployment.status) && Boolean(deployment.cancellationRequestedAt);
  // Steps and logs live in Deployment Mode's service panel; this row only names the attempt and holds its commands.
  return <>
    <article className="flex min-w-0 flex-col rounded-xl border">
      <header className="flex flex-wrap items-center gap-3 px-3 py-4 sm:gap-5 sm:px-5">
        <span className="rounded-md bg-muted/50 px-2.5 py-1.5 text-xs font-medium">{cancelling ? "Cancelling" : deploymentStatusLabel(attempt.view)}</span>
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium">{deployment.message ?? "Deployment"}</p>
          <p className="mt-1 text-xs text-muted-foreground">{deployment.projectSlug} / {deployment.environmentSlug} · {formatRelativeTime(deployment.createdAt)} · {deployment.serviceCount} {deployment.serviceCount === 1 ? "service" : "services"}</p>
        </div>
        {/* ponytail: temporary way into Deployment Mode; the deploy bar (#1050) replaces it. */}
        {organizationSlug ? <Button variant="outline" size="sm" nativeButton={false} render={<Link to={ENVIRONMENT_INDEX_ROUTE_TO} params={{ organizationSlug, projectSlug: deployment.projectSlug, environmentSlug: deployment.environmentSlug }} search={{ deployment: deployment.id }} />}>View on canvas</Button> : null}
        <DropdownMenu>
            <DropdownMenuTrigger
              render={<Button variant="ghost" size="icon" />}
            >
            <MoreVerticalIcon />
            <span className="sr-only">Deployment actions</span>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuGroup>
              {organizationSlug && ["queued", "planning", "deploying"].includes(deployment.status) ? (
                <DropdownMenuItem variant="destructive" onClick={() => setCancelOpen(true)}>
                  <XIcon />
                  Cancel deployment
                </DropdownMenuItem>
              ) : null}
              <DropdownMenuItem
                onClick={() =>
                  void navigator.clipboard.writeText(deployment.id)
                }
              >
                Copy deployment ID
              </DropdownMenuItem>
              {deployment.coreDeployId ? (
                <DropdownMenuItem
                  onClick={() =>
                    void navigator.clipboard.writeText(
                      deployment.coreDeployId ?? "",
                    )
                  }
                >
                  Copy core deploy ID
                </DropdownMenuItem>
              ) : null}
              {deployment.canRetry ? (
                <DropdownMenuItem
                  disabled={isRetrying}
                  onClick={() => void retryDeployment()}
                >
                  Retry deployment
                </DropdownMenuItem>
              ) : null}
                {queuedForNextTrigger ? (
                  <DropdownMenuItem
                    disabled={isDispatching}
                    onClick={() => void deployQueuedTarget()}
                  >
                    Deploy now
                  </DropdownMenuItem>
                ) : null}
              {!deployment.canRetry &&
              deployment.status === "failed" &&
              deployment.volumeRemoveAttempts.length > 0 ? (
                <DropdownMenuItem disabled>
                  Review volume deletion from the canvas
                </DropdownMenuItem>
              ) : null}
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </header>
      {deployment.volumeRemoveAttempts.length > 0 ? <div className="px-6 py-3"><VolumeRemoveAttemptHistory
        attempts={deployment.volumeRemoveAttempts} organizationSlug={organizationSlug ?? ""}
        projectSlug={deployment.projectSlug} environmentSlug={deployment.environmentSlug}
        availableVolumeResourceIds={availableVolumeResourceIds}
      /></div> : null}
    </article>
    {organizationSlug ? <CancelDeploymentDialog open={cancelOpen} onOpenChange={setCancelOpen} organizationSlug={organizationSlug} deployment={deployment} /> : null}
  </>;
}
