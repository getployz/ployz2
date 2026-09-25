import { useId, type ReactNode } from "react";
import { CheckIcon, ChevronDownIcon, CircleIcon, MinusIcon, TriangleAlertIcon, XIcon } from "lucide-react";
import { Spinner } from "#/components/ui/spinner";
import { Button } from "#/components/ui/button";
import { cn } from "#/lib/utils";
import type { EnvironmentDeploymentSummary } from "#/modules/deployments/deployment-contract";
import type { DeploymentProgress } from "#/modules/deployments/deployment-progress";
import { deploymentStatusLabel, deploymentView, nodeOutcomeLabels, type AttemptNode, type StageState } from "#/modules/deployments/deployment-view";
import { formatRelativeTime } from "#/utils/relative-time";

type StepState = "pending" | "running" | "completed" | "failed" | "skipped" | "unknown";
function Step({ title, detail, state, children }: { title: string; detail: string; state: StepState; children?: ReactNode }) {
  const Icon = state === "completed" ? CheckIcon : state === "failed" ? XIcon : state === "skipped" ? MinusIcon : state === "unknown" ? TriangleAlertIcon : CircleIcon;
  return <div className={cn("px-4 py-3 sm:px-6", state === "failed" && "bg-destructive/8 text-destructive")}>
    <div className="flex items-start gap-3 sm:gap-5">
      {state === "running" ? <Spinner className="mt-0.5 size-4 shrink-0" /> : <Icon className={cn("mt-0.5 size-4 shrink-0", state === "completed" ? "text-success" : state === "failed" ? "text-destructive" : "text-muted-foreground")} />}
      <div className="min-w-0 flex-1 text-sm"><span className="font-medium">{title}</span><span className={cn("ml-1", state !== "failed" && "text-muted-foreground")}> › {detail}</span>{children}</div>
      {state === "pending" || state === "skipped" ? <span className="shrink-0 text-xs text-muted-foreground">{state === "pending" ? "Not started" : "Skipped"}</span> : null}
    </div>
  </div>;
}
// Finishing-up work after the Deployment is terminal: deliberately quiet, never a status.
function ImageCleanupLine({ cleanup }: { cleanup: NonNullable<DeploymentProgress["imageCleanup"]> }) {
  const machines = `${cleanup.machines} ${cleanup.machines === 1 ? "Server" : "Servers"}`;
  const [icon, text] = {
    running: [<Spinner className="size-3" />, "Cleaning up old images…"],
    warning: [<TriangleAlertIcon className="size-3" />, `Old image cleanup incomplete · ${machines}`],
    cleaned: [<CheckIcon className="size-3" />, `Cleaned up old images · ${machines}`],
  }[cleanup.state];
  return <p className="flex items-center gap-2 px-4 py-1.5 text-xs text-muted-foreground sm:px-6">{icon}{text}</p>;
}

const stepStates = { queued: "pending", running: "running", done: "completed", failed: "failed", skipped: "skipped", none: "skipped", unknown: "unknown" } satisfies Record<StageState, StepState>;
const preparationLabels = { source: "Acquiring source", selection: "Selecting build Server", build: "Building images", transfer: "Transferring images", ready: "Images prepared" } as const;

// ponytail: approximates the Attempt Target's nodes from built services and progress rows until the read model exposes them (#1049).
function attemptNodes(deployment: EnvironmentDeploymentSummary, progress: DeploymentProgress | null, serviceId?: string): AttemptNode[] {
  const rowIds = (progress?.rows ?? []).flatMap((row) => row.serviceId ? [row.serviceId] : []);
  const ids = serviceId ? [serviceId] : [...new Set([...deployment.buildServiceIds, ...rowIds])];
  return ids.map((nodeId) => {
    const built = deployment.buildServiceIds.includes(nodeId);
    return { nodeId, built, changed: built || rowIds.includes(nodeId) || !progress?.outcome };
  });
}

export function DeploymentStatusCard({ deployment, progress, logsPanel, showLogs, onLogsChange, onLogsIntent, expanded, onExpandedChange, actions, children, serviceId }: {
  deployment: EnvironmentDeploymentSummary; progress: DeploymentProgress | null;
  logsPanel: ReactNode; showLogs: boolean; onLogsChange: (open: boolean) => void; onLogsIntent?: () => void; expanded: boolean; onExpandedChange: (open: boolean) => void;
  actions: ReactNode; children?: ReactNode; serviceId?: string;
}) {
  const id = useId();
  const view = deploymentView({ deployment, progress, nodes: attemptNodes(deployment, progress, serviceId) });
  const node = serviceId ? view.nodes[0] : undefined;
  const nodes = node ? [node] : view.nodes;
  const serviceName = (nodeId: string) => progress?.rows.find((r) => r.serviceId === nodeId)?.serviceName ?? nodeId;
  const active = ["queued", "planning", "deploying"].includes(deployment.status);
  const cancelling = active && Boolean(deployment.cancellationRequestedAt);
  const build = node?.build ?? view.build;
  const deploy = node?.deploy ?? view.deploy;
  const successful = node ? node.outcome === "deployed" || node.outcome === "removed" : view.status === "deployed";
  const failed = node ? node.outcome === "failed" : view.status === "failed";
  const unknown = build.state === "unknown" || deploy.state === "unknown";
  const preparation = progress?.preparation;
  const pins = Object.entries(deployment.sourcePins).filter(([id]) => !serviceId || id === serviceId);
  const badge = cancelling ? "Cancelling" : node ? nodeOutcomeLabels[node.outcome] : deploymentStatusLabel(view);
  const tone = failed ? "border-destructive/30 bg-destructive/4" : successful ? "border-success/30 bg-success/4" : active ? "border-info/30 bg-info/4" : "border-border bg-muted/20";
  const accent = failed ? "text-destructive" : successful ? "text-success" : active ? "text-info" : "text-muted-foreground";
  const waiting = preparation ? preparationLabels[preparation.phase] : "Waiting to prepare images";
  const buildDetail = { none: "Using prebuilt images", done: "Images prepared", unknown: "Preparation outcome unavailable", failed: deployment.failureMessage ?? "Image preparation failed", skipped: "Preparation cancelled", running: waiting, queued: waiting }[build.state];
  const deployDetail = node?.outcome === "unchanged" ? "No operations planned for this service"
    : !node && progress?.rows.length ? `${view.deployed} of ${view.changed} deployed`
    : { done: "Deployed", failed: "Deployment failed", unknown: "Runtime outcome unavailable", running: "Waiting for runtime progress", queued: "Waiting to start", skipped: "Not started", none: "Not started" }[deploy.state];
  const headline = cancelling ? "Cancelling deployment · waiting for runtime cleanup"
    : build.state === "unknown" ? "Preparation ended · outcome unknown"
    : deploy.state === "unknown" ? "Deployment ended · runtime outcome unknown"
    : node?.outcome === "unchanged" ? "No changes for this service"
    : node && ["deployed", "removed", "failed", "not_attempted"].includes(node.outcome) ? `Service ${nodeOutcomeLabels[node.outcome].toLowerCase()}`
    : view.status === "queued" ? (deployment.dispatchRequestedAt ? "Waiting to deploy" : "Queued for next trigger")
    : view.status === "deployed" ? "Deployment successful"
    : view.status === "failed" ? `Deployment ${deploymentStatusLabel(view).toLowerCase()}`
    : view.status === "cancelled" ? "Deployment cancelled"
    : "Deployment in progress";
  return <article className={cn("min-w-0 rounded-xl border p-1", tone)}>
    <header className="flex flex-wrap items-center gap-3 rounded-lg px-3 py-4 sm:gap-5 sm:px-5">
      <span className={cn("rounded-md bg-muted/50 px-2.5 py-1.5 text-xs font-medium", accent)}>{badge}</span>
      <div className="min-w-0 flex-1"><p className="truncate text-sm font-medium">{serviceId ? `${progress?.rows.find((r) => r.serviceId === serviceId)?.serviceName ?? "Service"} · ` : ""}{deployment.message ?? "Deployment"}</p><p className="mt-1 text-xs text-muted-foreground">{deployment.projectSlug} / {deployment.environmentSlug} · {formatRelativeTime(deployment.createdAt)}{!serviceId ? ` · ${deployment.serviceCount} ${deployment.serviceCount === 1 ? "service" : "services"}` : ""}</p></div>
      <Button variant="outline" size="sm" onPointerEnter={onLogsIntent} onFocus={onLogsIntent} onClick={() => { onLogsChange(!showLogs); onExpandedChange(true); }} aria-expanded={showLogs} aria-controls={`${id}-logs`}>{showLogs ? "Hide logs" : "View logs"}</Button>{actions}
    </header>
    <button type="button" className={cn("flex w-full items-center gap-3 rounded-md bg-muted/30 px-4 py-3 text-left text-sm focus-visible:outline-2 focus-visible:outline-ring sm:px-6", accent)} aria-expanded={expanded} aria-controls={`${id}-steps`} onClick={() => onExpandedChange(!expanded)}>
      {successful ? <CheckIcon className="size-4" /> : failed || unknown ? <TriangleAlertIcon className="size-4" /> : active && deployment.status !== "queued" ? <Spinner className="size-4" /> : null}
      <span className="flex-1">{headline}</span><ChevronDownIcon className={cn("size-4", expanded && "rotate-180")} />
    </button>
    {expanded ? <div id={`${id}-steps`} className="rounded-b-lg bg-background py-2">
      <Step title="Build" state={stepStates[build.state]} detail={buildDetail}>
        {pins.map(([id, pin]) => <p key={id} className="mt-1 break-all font-mono text-xs">Commit: {pin.commitSha}</p>)}
        {build.state !== "none" && (preparation?.machineName || preparation?.machineId) ? <p className="mt-1 text-xs text-muted-foreground">Build Server: {preparation.machineName ?? preparation.machineId}</p> : null}
      </Step>
      <Step title="Deploy" state={stepStates[deploy.state]} detail={deployDetail}>
        {!node && nodes.length > 1 ? <div className="mt-3 space-y-2">{nodes.map((n) => <p key={n.nodeId} className="flex flex-wrap justify-between gap-2 text-xs"><span>{serviceName(n.nodeId)}</span><span className="text-muted-foreground">{nodeOutcomeLabels[n.outcome]}</span></p>)}</div> : null}
        {nodes.filter((n) => !n.failure).flatMap((n) => n.tail.map((line) => <p key={`${n.nodeId}:${line}`} className="mt-2 text-xs text-muted-foreground">{serviceName(n.nodeId)} · {line}</p>))}
        {nodes.flatMap((n) => n.failure ? [<p key={n.nodeId} className="mt-2 break-words font-mono text-xs">{serviceName(n.nodeId)} · {n.tail.join(" · ")}{n.failure.containerId ? ` · container ${n.failure.containerId}` : ""}</p>] : [])}
        {progress?.compensation.length ? <div className="mt-3 border-l border-primary/30 pl-3 text-xs"><p className="font-medium">Recovery reported by runtime</p>{progress.compensation.map((line) => <p key={line} className="mt-1">{line}</p>)}</div> : null}
        {deploy.state === "unknown" ? <p className="mt-2 text-xs text-muted-foreground">A complete runtime outcome was not received. Completed operations remain recorded; remaining effects are unknown.</p> : null}
      </Step>
      {progress?.imageCleanup ? <ImageCleanupLine cleanup={progress.imageCleanup} /> : null}
      {deployment.deployPreview?.warnings.length ? <details className="mx-6 my-2 text-xs"><summary className="cursor-pointer text-warning">{deployment.deployPreview.warnings.length} planning warnings</summary>{deployment.deployPreview.warnings.map((w, i) => <p key={i} className="mt-2 break-words">{JSON.stringify(w)}</p>)}</details> : null}
      {children}
    </div> : null}
    {progress?.logsIncomplete ? <p className="px-4 py-2 text-sm text-muted-foreground sm:px-6">Logs incomplete. Some progress or output could not be recorded.</p> : null}
    {showLogs ? <section id={`${id}-logs`} className="mt-1">{logsPanel}</section> : null}
  </article>;
}
