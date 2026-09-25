import { Handle, Position } from "@xyflow/react";
import { CheckIcon, CircleHelpIcon, CircleIcon, HardDriveIcon, XIcon } from "lucide-react";
import { parseServiceConfig } from "@ployz/sdk/config";
import { formatDuration } from "#/components/deployment-logs";
import { Avatar, AvatarFallback } from "#/components/ui/avatar";
import { Badge } from "#/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "#/components/ui/card";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/utils";
import { nodeOutcomeLabels, type DeploymentNodeView, type Stage } from "#/modules/deployments/deployment-view";
import { getServiceIcon, getServiceSubtitle } from "./service-node-helpers";
import type { CanvasDeploymentNodeData } from "./types";

export const outcomeBadges = {
  deployed: "success", removed: "secondary", failed: "destructive", not_attempted: "secondary", unchanged: "secondary",
  queued: "secondary", building: "info", deploying: "info",
} as const satisfies Record<DeploymentNodeView["outcome"], "success" | "secondary" | "destructive" | "info">;
const outcomeCards = {
  deployed: "success", removed: undefined, failed: "destructive", not_attempted: undefined, unchanged: undefined,
  queued: undefined, building: "info", deploying: "info",
} as const satisfies Record<DeploymentNodeView["outcome"], "success" | "destructive" | "info" | undefined>;

const stageIcons = {
  done: <CheckIcon className="size-3" aria-label="done" />,
  failed: <XIcon className="size-3" aria-label="failed" />,
  running: <Spinner className="size-3" aria-label="running" />,
  queued: <CircleIcon className="size-3" aria-label="queued" />,
  unknown: <CircleHelpIcon className="size-3" aria-label="unknown" />,
};

/** One stage of Build → Deploy: its state and, once finished, its duration. "—" when it has nothing to do. */
function StageLabel({ name, stage }: { name: string; stage: Stage }) {
  if (stage.state === "none" || stage.state === "skipped") return <span data-stage={name}>{name} —</span>;
  return (
    <span data-stage={name} className={cn("inline-flex items-center gap-1", stage.state === "done" && "text-success", stage.state === "failed" && "text-destructive", stage.state === "running" && "text-info")}>
      {stageIcons[stage.state]}{name}{stage.durationMs === undefined ? null : ` ${formatDuration(stage.durationMs)}`}
    </span>
  );
}

/**
 * A node's card in Deployment Mode: read-only, badged with its Node Outcome, with Build → Deploy and a two-line tail.
 * Unchanged and Not attempted nodes show only their name and outcome, dimmed.
 */
export function DeploymentNodeCard({ data, className }: { data: CanvasDeploymentNodeData; className?: string }) {
  const { view } = data;
  const source = data.nodeType === "service" ? parseServiceConfig(data.config).source : null;
  const subtitle = source ? getServiceSubtitle({ source }) : "Named volume";
  const dimmed = view.outcome === "unchanged" || view.outcome === "not_attempted";
  return (
    <Card size="node" state={outcomeCards[view.outcome]}
      data-canvas-node={data.nodeId} data-dimmed={dimmed} className={cn("justify-between", dimmed && "opacity-40", className)}>
      <CardHeader>
        <div className="flex items-start gap-3">
          <Avatar>
            <AvatarFallback>{source ? getServiceIcon({ source }) : <HardDriveIcon />}</AvatarFallback>
          </Avatar>
          <div className="min-w-0 flex-1 overflow-hidden">
            <CardTitle className="truncate">{data.name}</CardTitle>
            {subtitle && !dimmed ? <CardDescription className="truncate">{subtitle}</CardDescription> : null}
          </div>
          <Badge variant={outcomeBadges[view.outcome]}>{nodeOutcomeLabels[view.outcome]}</Badge>
        </div>
      </CardHeader>
      {dimmed ? null : (
        <CardContent className="flex flex-col gap-2">
          <div className="flex items-center gap-1.5 whitespace-nowrap text-muted-foreground">
            <StageLabel name="Build" stage={view.build} />
            <span aria-hidden className="opacity-50">→</span>
            <StageLabel name="Deploy" stage={view.deploy} />
          </div>
          {view.tail.length ? (
            <div data-tail className="rounded-md bg-muted px-2 py-1 font-mono text-[10.5px] leading-normal text-muted-foreground">
              {view.tail.map((line, index) => (
                <div key={index} className={cn("truncate", view.failure ? "text-destructive" : index === view.tail.length - 1 && "text-foreground")}>{line}</div>
              ))}
            </div>
          ) : null}
        </CardContent>
      )}
    </Card>
  );
}

export function DeploymentNode({ data }: { data: CanvasDeploymentNodeData }) {
  return (
    <div className="min-h-36 w-72">
      <Handle type="target" position={Position.Bottom} isConnectable={false} className="opacity-0" />
      <Handle type="source" position={Position.Top} isConnectable={false} className="opacity-0" />
      <DeploymentNodeCard data={data} className="min-h-36" />
    </div>
  );
}
