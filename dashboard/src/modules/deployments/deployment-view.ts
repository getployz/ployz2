import type { DeployEvent, DeployOperation, OperationRow } from "@ployz/sdk";
import { parseServiceConfig } from "@ployz/sdk/config";
import { canonicalJson } from "#/modules/environment-design/canonical-json";
import type { EnvironmentDeploymentStatus } from "./tables";
import { BUILDING_KEY } from "./preparation-progress";
import { executionErrorLabel, progressRowLabel, type DeploymentProgress, type DeploymentProgressRow } from "./deployment-progress";

/**
 * The deployment view projection: the only place deployment state rules live.
 * Record side: `deploymentProgressForEvent` folds Engine events into durable progress.
 * Read side: `deploymentView` turns an attempt's data into what the UI renders.
 */

export type NodeOutcome = "deployed" | "removed" | "failed" | "not_attempted" | "unchanged";
/** `none` = prebuilt image; `unknown` = the attempt ended without a complete outcome. */
export type StageState = "queued" | "running" | "done" | "failed" | "skipped" | "none" | "unknown";
export type Stage = { state: StageState; durationMs?: number };
export type DeploymentNodeView = {
  nodeId: string;
  outcome: NodeOutcome | "queued" | "building" | "deploying";
  build: Stage;
  deploy: Stage;
  failure: { message: string; containerId: string | null } | null;
  tail: string[];
};
export type DeploymentViewStatus = "queued" | "building" | "deploying" | "deployed" | "failed" | "cancelled";
export type DeploymentView = {
  status: DeploymentViewStatus;
  /** Whole-attempt stages; per-node stages live on each node. */
  build: Stage;
  deploy: Stage;
  deployed: number; changed: number;
  nodes: DeploymentNodeView[];
};
/** One Environment Node of the Attempt Target. `image` names its Image Build (the service's private DNS name) when built. */
export type AttemptNode = { nodeId: string; changed: boolean; removed?: boolean; built?: boolean; image?: string };
/** The attempt's Build Steps and their output, as the build log read returns them. */
export type BuildLog = {
  steps: readonly { id: number; build: number; key: string; name: string; startedAt: Date | null; completedAt: Date | null; error: string | null }[];
  output: readonly { stepId: number; text: string }[];
};
export type DeploymentViewInput = {
  deployment: {
    status: EnvironmentDeploymentStatus;
    failureCode: string | null;
    failureMessage: string | null;
    deployPreview: unknown;
  };
  progress: DeploymentProgress | null;
  nodes: readonly AttemptNode[];
  buildLog?: BuildLog | null;
};

// Provider messages and operation specs may contain resolved secrets. Only
// explicit identity, lifecycle and structured failure facts cross this boundary.
function projectRow(row: OperationRow, serviceId: string | null): DeploymentProgressRow {
  const op = row.operation;
  const phase = row.status.type === "running" ? row.status.phase : null;
  const timed = phase?.type === "waiting_for_health" || phase?.type === "waiting_for_hook" ? phase : null;
  const target = "container_id" in op ? op.container_id : "old_container_id" in op ? op.old_container_id : op.type === "remove_volume" ? op.id.name : null;
  const error = row.status.type === "failed" ? row.status.error : null;
  const containerId = error?.type === "health" || error?.type === "hook" ? error.container_id : error && "container_id" in op ? op.container_id : null;
  return {
    index: row.index, machineId: row.machine_id, machineName: row.machine_name,
    serviceId,
    runtimeServiceId: "spec" in op ? op.spec.service_id : null,
    serviceName: row.service_name, displayName: row.display_name,
    operation: op.type,
    target,
    updateOrder: op.type === "replace_container" ? op.spec.update.order : null,
    status: row.status.type, phase: phase?.type ?? null,
    elapsedMs: timed?.elapsed_ms ?? null, deadlineMs: timed?.deadline_ms ?? null,
    health: phase?.type === "waiting_for_health" ? phase.health : null,
    error: error ? executionErrorLabel(error) : null,
    containerId,
  };
}

/**
 * Operations are matched structurally: the Engine serializes keys in its own
 * (alphabetical) order, so raw JSON of the planned and reported operation differs.
 * A failed row keeps the phase and deadline it was last seen running with.
 */
export function deploymentProgressForEvent(
  event: Exclude<DeployEvent, { type: "images_pruned" }>,
  planned: readonly OperationRow[],
  context: { prior?: DeploymentProgress; serviceIdFor?: (serviceName: string | null) => string | null; now?: number } = {},
): DeploymentProgress {
  const now = context.now ?? Date.now();
  const project = (row: OperationRow) => {
    const projected = projectRow(row, context.serviceIdFor?.(row.service_name) ?? null);
    const prior = context.prior?.rows.find((candidate) => candidate.index === row.index);
    const finished = projected.status === "completed" || projected.status === "failed";
    const timed = { ...projected,
      startedAt: prior?.startedAt ?? (finished || projected.status === "running" ? now : null),
      finishedAt: prior?.finishedAt ?? (finished ? now : null) };
    return projected.status === "failed" && prior ? { ...timed, phase: prior.phase, elapsedMs: prior.elapsedMs, deadlineMs: prior.deadlineMs, health: prior.health } : timed;
  };
  if (event.type === "progress") return { completed: event.completed, total: event.total, rows: event.rows.map(project), outcome: null, compensation: [] };
  const outcome = event.outcome;
  const completed = outcome.completed.map((op) => canonicalJson(op));
  const failedOp: DeployOperation | null = outcome.type === "failed"
    ? outcome.failed.type === "replacement_health" ? { type: "replace_container", ...outcome.failed.operation } : outcome.failed.operation
    : null;
  const failedKey = failedOp && canonicalJson(failedOp);
  let failureAssigned = false;
  const rows = planned.map((row) => {
    const key = canonicalJson(row.operation);
    const completedIndex = completed.indexOf(key);
    if (completedIndex !== -1) {
      completed.splice(completedIndex, 1);
      return project({ ...row, status: { type: "completed" } });
    }
    if (outcome.type === "failed" && key === failedKey && !failureAssigned) {
      failureAssigned = true;
      return project({ ...row, status: { type: "failed", error: outcome.failed.error } });
    }
    return project({ ...row, status: { type: "unexecuted" } });
  });
  const compensation: string[] = [];
  if (outcome.type === "failed" && outcome.failed.type === "replacement_health") {
    const c = outcome.failed.compensation;
    compensation.push(c.stop_new_container.type === "stopped" ? "Replacement container stopped" : `Could not stop replacement: ${executionErrorLabel(c.stop_new_container.error)}`);
    if (c.type === "stop_first") {
      compensation.push(c.restart_old_container.type === "restarted" ? "Previous container restarted" : c.restart_old_container.type === "not_attempted" ? "Previous container restart not attempted" : `Previous container restart failed: ${executionErrorLabel(c.restart_old_container.error)}`);
    }
  }
  return { completed: outcome.completed.length, total: rows.length, rows, outcome: outcome.type, compensation };
}

const TAIL_LINES = 2;
const rowLine = (row: DeploymentProgressRow) => `${row.machineName ?? row.machineId} · ${row.health ?? progressRowLabel(row)}${row.status === "completed" ? " · done" : ""}${row.elapsedMs !== null ? ` · ${Math.floor(row.elapsedMs / 1000)}s / ${Math.floor((row.deadlineMs ?? 0) / 1000)}s deadline` : ""}`;

/** Terminal colour and cursor sequences carry nothing the log needs. */
const ESC = String.fromCharCode(27);
const BEL = String.fromCharCode(7);
const ansi = new RegExp(`${ESC}(?:\\[[0-?]*[ -/]*[@-~]|\\][^${BEL}]*(?:${BEL}|${ESC}\\\\)|[@-Z\\\\-_])`, "g");
export const stripAnsi = (text: string) => text.replaceAll(ansi, "");
const logLines = (text: string) => stripAnsi(text).split("\n").filter((line) => line.trim());

/**
 * One Image Build: the build runs whose Building heading names the image (a multi-platform image has several),
 * without the attempt-wide cleanup and delivery that land under the last run.
 */
function imageBuild(log: BuildLog, image: string) {
  const runs = new Set(log.steps.filter((step) => step.key === BUILDING_KEY && step.name === image).map((step) => step.build));
  const steps = log.steps.filter((step) => runs.has(step.build) && step.key !== "stage:Cleanup" && step.key !== "transfer");
  if (!steps.length) return null;
  const failed = steps.filter((step) => step.error !== null);
  const output = (ids: ReadonlySet<number>) => logLines(log.output.filter((row) => ids.has(row.stepId)).map((row) => row.text).join(""));
  const starts = steps.flatMap((step) => step.startedAt ? [step.startedAt.getTime()] : []);
  const ends = steps.flatMap((step) => step.completedAt ? [step.completedAt.getTime()] : []);
  return {
    failed: failed.length > 0,
    open: steps.some((step) => step.startedAt && !step.completedAt),
    durationMs: starts.length && ends.length ? Math.max(...ends) - Math.min(...starts) : undefined,
    lines: output(new Set(steps.map((step) => step.id))),
    errorLines: [...output(new Set(failed.map((step) => step.id))), ...failed.flatMap((step) => logLines(step.error ?? ""))],
  };
}

export function deploymentView({ deployment, progress, nodes, buildLog }: DeploymentViewInput): DeploymentView {
  const { status } = deployment;
  const active = status === "queued" || status === "planning" || status === "deploying";
  const succeeded = progress?.outcome === "success" || status === "applied";
  const hasBuild = nodes.some((node) => node.built);
  const buildState: StageState = !hasBuild ? "none"
    : progress?.preparation?.phase === "ready" || deployment.deployPreview || succeeded ? "done"
    : !active && deployment.failureCode === "sdk_preparation_unknown" ? "unknown"
    : status === "failed" ? "failed"
    : status === "cancelled" ? "skipped"
    : progress?.preparation && active ? "running" : "queued";
  const ready = buildState === "none" || buildState === "done";
  const runtimeUnknown = !active && status !== "applied" && !progress?.outcome && deployment.failureCode === "sdk_deploy_outcome_unknown";
  const rows = progress?.rows ?? [];
  const images = new Map(nodes.map((node) => [node.nodeId, buildLog && node.image ? imageBuild(buildLog, node.image) : null]));
  const building = [...images.values()].some(Boolean);
  const blamed = [...images.values()].some((image) => image?.failed);

  const views = nodes.map((node): DeploymentNodeView => {
    const own = rows.filter((row) => row.serviceId === node.nodeId);
    const failedRow = own.find((row) => row.status === "failed");
    const image = images.get(node.nodeId) ?? null;
    // Images build one after another: one without a run yet waits, and is skipped once another image's build failed.
    const build: Stage = !node.built ? { state: "none" }
      : !image ? { state: buildState === "running" && building ? "queued" : buildState === "failed" && blamed ? "skipped" : buildState }
      : image.failed ? { state: "failed", durationMs: image.durationMs }
      : !image.open || buildState === "done" ? { state: "done", durationMs: image.durationMs }
      : { state: active ? "running" : buildState };
    const done = (own.length > 0 && own.every((row) => row.status === "completed")) || succeeded;
    // Without a runtime outcome, a failed attempt blames the image whose build failed, or else every changed node it could have reached.
    const preRuntimeFailure = status === "failed" && !progress?.outcome && (blamed ? build.state === "failed" : node.built || ready);
    const deployState: StageState = !node.changed ? "skipped"
      : failedRow ? "failed"
      : done ? "done"
      : runtimeUnknown ? "unknown"
      : !active ? preRuntimeFailure && ready ? "failed" : "skipped"
      : own.some((row) => row.status === "running") ? "running"
      : "queued";
    const failure = failedRow ? { message: failedRow.error ?? "Deployment failed", containerId: failedRow.containerId ?? null }
      : node.changed && !done && preRuntimeFailure ? { message: deployment.failureMessage ?? (build.state === "failed" ? "Image preparation failed" : "Deployment failed"), containerId: null }
      : null;
    const outcome: DeploymentNodeView["outcome"] = !node.changed ? "unchanged"
      : failure ? "failed"
      : done ? node.removed ? "removed" : "deployed"
      : !active ? "not_attempted"
      : build.state === "running" ? "building"
      : deployState === "running" ? "deploying"
      : "queued";
    // The tail follows the stage that matters: the error when failed, else the rollout once it started, else the build.
    const started = own.filter((row) => row.status === "running" || row.status === "completed");
    const tail = failure ? failedRow ? [`${failedRow.machineName ?? failedRow.machineId} · ${failure.message}`]
        : build.state === "failed" && image?.errorLines.length ? image.errorLines : [failure.message]
      : started.length ? started.map(rowLine)
      : image?.lines ?? [];
    const spans = own.flatMap((row) => row.startedAt != null && row.finishedAt != null ? [[row.startedAt, row.finishedAt] as const] : []);
    const deployDurationMs = (deployState === "done" || deployState === "failed") && spans.length
      ? Math.max(...spans.map(([, end]) => end)) - Math.min(...spans.map(([start]) => start)) : undefined;
    return { nodeId: node.nodeId, outcome, build, deploy: { state: deployState, durationMs: deployDurationMs }, failure, tail: tail.slice(-TAIL_LINES) };
  });

  const changed = views.filter((node) => node.outcome !== "unchanged");
  const deployState: StageState = changed.some((node) => node.deploy.state === "failed") ? "failed"
    : succeeded ? "done"
    : runtimeUnknown ? "unknown"
    : status === "deploying" && ready ? "running"
    : status === "failed" && ready ? "failed"
    : active ? "queued" : "skipped";
  return {
    status: status === "applied" ? "deployed" : status === "failed" || status === "cancelled" || status === "queued" ? status
      : ready ? "deploying" : "building",
    build: { state: buildState },
    deploy: { state: deployState },
    deployed: changed.filter((node) => node.outcome === "deployed" || node.outcome === "removed").length,
    changed: changed.length,
    nodes: views,
  };
}

type AttemptRow = { id: string; status: EnvironmentDeploymentStatus; createdAt: Date };
type SnapshotRow = { environmentDeploymentId: string; nodeType: "service" | "volume"; nodeId: string; config: unknown };
/** An Attempt Target node with the configuration it was deployed (or last deployed, when removed) with. */
export type AttemptTargetNode = AttemptNode & { nodeType: "service" | "volume"; config: unknown };

/**
 * The Attempt Target's full node set, read from deployment snapshots: every node the attempt froze,
 * plus the nodes the last applied attempt before it had and this one dropped (Removed).
 * Once the Engine reports rows, a service is changed only if it has operations; before that,
 * a node is changed when it is built or its snapshot differs from that applied attempt's.
 * Rows for removed services carry no serviceId from the record side, so they are resolved here by name.
 */
export function attemptTarget({ attempt, progress, history, snapshots }: {
  attempt: AttemptRow; progress: DeploymentProgress | null;
  history: readonly AttemptRow[]; snapshots: readonly SnapshotRow[];
}) {
  // ponytail: diffs against the last fully applied attempt; a failed attempt in between that deployed some nodes is ignored until rows arrive.
  const base = history.filter((row) => row.status === "applied" && row.createdAt < attempt.createdAt)
    .sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime())[0];
  const own = snapshots.filter((row) => row.environmentDeploymentId === attempt.id);
  const prior = base ? snapshots.filter((row) => row.environmentDeploymentId === base.id) : [];
  const removed = prior.filter((row) => !own.some((node) => node.nodeType === row.nodeType && node.nodeId === row.nodeId));
  const services = [...own, ...removed].flatMap((row) => row.nodeType === "service" ? [{ nodeId: row.nodeId, config: parseServiceConfig(row.config) }] : []);
  const resolved = progress && { ...progress, rows: progress.rows.map((row) => row.serviceId ? row
    : { ...row, serviceId: services.find((s) => s.config.privateDns === row.serviceName)?.nodeId ?? null }) };
  const rows = resolved?.rows ?? [];
  const nodes = own.map((row): AttemptTargetNode => {
    const config = services.find((s) => s.nodeId === row.nodeId)?.config;
    const built = config?.source.type === "git";
    const before = prior.find((node) => node.nodeType === row.nodeType && node.nodeId === row.nodeId);
    const changed = row.nodeType === "service" && rows.length > 0 ? rows.some((r) => r.serviceId === row.nodeId)
      : built || !before || canonicalJson(before.config) !== canonicalJson(row.config);
    return { nodeId: row.nodeId, nodeType: row.nodeType, config: row.config, changed, built, image: built ? config?.privateDns : undefined };
  });
  return { nodes: [...nodes, ...removed.map((row) => ({ nodeId: row.nodeId, nodeType: row.nodeType, config: row.config, changed: true, removed: true }))], progress: resolved };
}

/** User-facing whole-deployment status: "Deployed" or "Failed · 2 of 4 deployed". Never "Partial". */
export function deploymentStatusLabel(view: DeploymentView): string {
  if (view.status === "failed") return `Failed · ${view.deployed} of ${view.changed} deployed`;
  return { queued: "Queued", building: "Building", deploying: "Deploying", deployed: "Deployed", cancelled: "Cancelled" }[view.status];
}

export const nodeOutcomeLabels = {
  deployed: "Deployed", removed: "Removed", failed: "Failed", not_attempted: "Not attempted", unchanged: "Unchanged",
  queued: "Queued", building: "Building", deploying: "Deploying",
} satisfies Record<DeploymentNodeView["outcome"], string>;
