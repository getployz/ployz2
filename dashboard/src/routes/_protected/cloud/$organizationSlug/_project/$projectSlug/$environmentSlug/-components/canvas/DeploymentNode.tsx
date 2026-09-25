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

export function DeploymentNode({ data }: { data: CanvasDeploymentNodeData }) {
  return (
    <div className="h-36 w-72">
      <Handle type="target" position={Position.Bottom} isConnectable={false} className="opacity-0" />
      <Handle type="source" position={Position.Top} isConnectable={false} className="opacity-0" />
      <DeploymentNodeCard data={data} className="h-full" />
    </div>
  );
}
