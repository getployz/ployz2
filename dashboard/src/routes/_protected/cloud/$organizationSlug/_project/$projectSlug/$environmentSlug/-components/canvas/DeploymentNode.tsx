import type { ReactNode } from "react";
import { Link, useParams } from "@tanstack/react-router";
import { Handle, Position } from "@xyflow/react";
import { HardDriveIcon } from "lucide-react";
import { parseServiceConfig } from "@ployz/sdk/config";
import { Avatar, AvatarFallback } from "#/components/ui/avatar";
import { Badge } from "#/components/ui/badge";
import { Card, CardDescription, CardHeader, CardTitle } from "#/components/ui/card";
import { cn } from "#/lib/utils";
import { nodeOutcomeLabels, type DeploymentNodeView } from "#/modules/deployments/deployment-view";
import { getServiceIcon, getServiceSubtitle } from "./service-node-helpers";
import type { CanvasDeploymentNodeData } from "./types";
import { ENVIRONMENT_ROUTE_FROM, ENVIRONMENT_SERVICE_ROUTE_TO } from "../environment-route-paths";
import { useDeploymentMode } from "../deployment-mode";
import { useCollectionScope } from "#/collections/use-collection-scope";
import { preloadDeploymentLogs } from "#/modules/deployments/deployment-log.collection";

export const outcomeBadges = {
  deployed: "success", removed: "secondary", failed: "destructive", not_attempted: "secondary", unchanged: "secondary",
  queued: "secondary", building: "info", deploying: "info",
} as const satisfies Record<DeploymentNodeView["outcome"], "success" | "secondary" | "destructive" | "info">;

/** A node's card in Deployment Mode: read-only, badged with its Node Outcome, dimmed when the attempt did not change it. */
export function DeploymentNodeCard({ data, className }: { data: CanvasDeploymentNodeData; className?: string }) {
  const source = data.nodeType === "service" ? parseServiceConfig(data.config).source : null;
  const subtitle = source ? getServiceSubtitle({ source }) : "Named volume";
  const dimmed = data.view.outcome === "unchanged" || data.view.outcome === "not_attempted";
  return (
    <Card size="node" data-canvas-node={data.nodeId} data-dimmed={dimmed} className={cn("justify-between", dimmed && "opacity-40", className)}>
      <CardHeader>
        <div className="flex items-start gap-3">
          <Avatar>
            <AvatarFallback>{source ? getServiceIcon({ source }) : <HardDriveIcon />}</AvatarFallback>
          </Avatar>
          <div className="min-w-0 flex-1 overflow-hidden">
            <CardTitle className="truncate">{data.name}</CardTitle>
            {subtitle ? <CardDescription className="truncate">{subtitle}</CardDescription> : null}
          </div>
          <Badge variant={outcomeBadges[data.view.outcome]}>{nodeOutcomeLabels[data.view.outcome]}</Badge>
        </div>
      </CardHeader>
    </Card>
  );
}

/** Service nodes open the read-only Deployment Mode panel; the retained `deployment` param keeps the mode. */
export function DeploymentNodeLink({ data, className, children }: { data: CanvasDeploymentNodeData; className?: string; children: ReactNode }) {
  const params = useParams({ from: ENVIRONMENT_ROUTE_FROM });
  const scope = useCollectionScope();
  const attempt = useDeploymentMode();
  if (data.nodeType !== "service") return <div className={className}>{children}</div>;
  const warm = () => { if (attempt) preloadDeploymentLogs(params.organizationSlug, attempt.deployment.id, scope); };
  return <Link to={ENVIRONMENT_SERVICE_ROUTE_TO} params={{ ...params, serviceId: data.nodeId }} search={(previous) => ({ ...previous, tab: undefined })}
    onPointerEnter={warm} onFocus={warm} data-canvas-node={data.nodeId} draggable={false} className={cn("block", className)}>{children}</Link>;
}

export function DeploymentNode({ data }: { data: CanvasDeploymentNodeData }) {
  return (
    <DeploymentNodeLink data={data} className="h-36 w-72">
      <Handle type="target" position={Position.Bottom} isConnectable={false} className="opacity-0" />
      <Handle type="source" position={Position.Top} isConnectable={false} className="opacity-0" />
      <DeploymentNodeCard data={data} className="h-full" />
    </DeploymentNodeLink>
  );
}
