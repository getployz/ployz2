/* oxlint-disable -- PROTOTYPE: throwaway fake-data UI on prototype/build-order-dashboard, never merged. */
// PROTOTYPE (prototype/build-order-dashboard): a fake Cloud Deployment Attempt at `?deployment=fake`,
// replaying how Image Builds walk the Build Order. No backend.
import { useEffect, useState } from "react";
import { parseServiceConfig } from "@ployz/sdk/config";
import { RotateCcwIcon } from "lucide-react";
import { Button } from "#/components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "#/components/ui/toggle-group";
import type { DeploymentAttempt } from "#/modules/deployments/deployment.collection";
import type { AttemptTargetNode, DeploymentNodeView, Stage } from "#/modules/deployments/deployment-view";
import type { EnvironmentDeploymentSummary } from "#/modules/deployments/deployment-contract";

export const FAKE_DEPLOYMENT_ID = "fake";

/** Where an Image Build ran and why; shown on the node and in the panel. */
export type BuilderEvidence = {
  builder: string;
  reason: string;
  skipped: string[];
  runUrl?: string;
  waiting?: string;
};

type Scenario = "normal" | "github-stuck" | "build-fails";
const SCENARIO_LABELS: Record<Scenario, string> = {
  normal: "Normal",
  "github-stuck": "GitHub never starts",
  "build-fails": "One build fails",
};

const git = (repository: string) => ({
  version: 2, type: "git", repository, repositoryId: 42, access: { type: "public" }, rootDir: "/",
  branch: { type: "connected", name: "main" },
});
const service = (nodeId: string, privateDns: string, source: unknown, image: string | null = null): AttemptTargetNode => ({
  nodeId, changed: true, removed: false, image, nodeType: "service",
  config: parseServiceConfig({
    version: 2, source, healthcheck: { type: "none" }, restartPolicy: "unless-stopped", privateDns,
    ...(image === null ? { build: { buildMethod: "dockerfile", dockerfilePath: "Dockerfile", command: null } } : {}),
  }),
});
const NODES: AttemptTargetNode[] = [
  service("proto-web", "web", git("acme/shop")),
  service("proto-api", "api", git("acme/api")),
  service("proto-worker", "worker", git("acme/shop")),
  service("proto-postgres", "postgres", { version: 1, type: "image", image: "postgres:17", credentials: { type: "none" } }, "postgres:17"),
];

// One plan per Image Build: seconds to wait for the builder, seconds to build, and what the log says.
type Plan = { wait: number; build: number; fails?: boolean; evidence: BuilderEvidence; waitingLine: string; buildLines: string[] };
const DOCKER = ["[internal] load build definition", "RUN pnpm install --frozen-lockfile", "RUN pnpm build", "exporting image"];
const PLANS: Record<Scenario, Record<string, Plan>> = {
  normal: {
    "proto-web": { wait: 3, build: 6, waitingLine: "waiting for a GitHub runner…", buildLines: [...DOCKER, "pushed to hel-1"],
      evidence: { builder: "GitHub Actions", reason: "first in build order", skipped: [], runUrl: "https://github.com/acme/shop/actions/runs/412" } },
    "proto-api": { wait: 0, build: 5, waitingLine: "", buildLines: DOCKER,
      evidence: { builder: "nuc-basement", reason: "had api's build cache", skipped: ["GitHub: no workflow in acme/api"] } },
    "proto-worker": { wait: 3, build: 5, waitingLine: "waiting for a GitHub runner…", buildLines: [...DOCKER, "pushed to hel-1"],
      evidence: { builder: "GitHub Actions", reason: "first in build order", skipped: [], runUrl: "https://github.com/acme/shop/actions/runs/413" } },
  },
  "github-stuck": {
    "proto-web": { wait: 6, build: 4, waitingLine: "no GitHub runner yet · 2m 40s left", buildLines: DOCKER,
      evidence: { builder: "nuc-basement", reason: "had web's build cache", skipped: ["GitHub: no runner started within 3 min"] } },
    "proto-api": { wait: 0, build: 5, waitingLine: "", buildLines: DOCKER,
      evidence: { builder: "nuc-basement", reason: "had api's build cache", skipped: ["GitHub: no workflow in acme/api"] } },
    "proto-worker": { wait: 6, build: 4, waitingLine: "no GitHub runner yet · 2m 40s left", buildLines: DOCKER,
      evidence: { builder: "hel-1", reason: "had worker's build cache", skipped: ["GitHub: no runner started within 3 min"] } },
  },
  "build-fails": {
    "proto-web": { wait: 3, build: 6, waitingLine: "waiting for a GitHub runner…", buildLines: [...DOCKER, "pushed to hel-1"],
      evidence: { builder: "GitHub Actions", reason: "first in build order", skipped: [], runUrl: "https://github.com/acme/shop/actions/runs/412" } },
    "proto-api": { wait: 0, build: 3, fails: true, waitingLine: "", buildLines: ["[internal] load build definition", "RUN pnpm install --frozen-lockfile", "ERR_PNPM_OUTDATED_LOCKFILE: lockfile out of date"],
      evidence: { builder: "nuc-basement", reason: "had api's build cache", skipped: ["GitHub: no workflow in acme/api"] } },
    "proto-worker": { wait: 3, build: 5, waitingLine: "waiting for a GitHub runner…", buildLines: [...DOCKER, "pushed to hel-1"],
      evidence: { builder: "GitHub Actions", reason: "first in build order", skipped: [], runUrl: "https://github.com/acme/shop/actions/runs/413" } },
  },
};
const DEPLOY_SECONDS = 3;

let scenario: Scenario = "normal";
let startedAt = Date.now();
const listeners = new Set<() => void>();
function restart(next: Scenario) {
  scenario = next;
  startedAt = Date.now();
  for (const listener of listeners) listener();
}

function nodeView(nodeId: string, t: number, deployStart: number | null, attemptFailed: boolean): DeploymentNodeView & { evidence?: BuilderEvidence } {
  const plan = PLANS[scenario][nodeId];
  const deploy: Stage = attemptFailed ? { state: "skipped" }
    : deployStart === null || t < deployStart ? { state: "queued" }
    : t < deployStart + DEPLOY_SECONDS ? { state: "running" }
    : { state: "done", durationMs: DEPLOY_SECONDS * 60_000 };
  const deployOutcome = deploy.state === "done" ? "deployed" : deploy.state === "running" ? "deploying" : attemptFailed ? "not_attempted" : "queued";
  if (!plan) return { nodeId, outcome: deployOutcome, build: { state: "none" }, deploy, failure: null, tail: [] };
  const buildEnd = plan.wait + plan.build;
  if (t < plan.wait) {
    return { nodeId, outcome: "building", build: { state: "queued" }, deploy, failure: null, tail: [plan.waitingLine],
      evidence: { ...plan.evidence, builder: plan.evidence.runUrl ? "GitHub Actions" : plan.evidence.builder, waiting: plan.waitingLine } };
  }
  if (t < buildEnd) {
    const shown = Math.max(1, Math.ceil(((t - plan.wait) / plan.build) * plan.buildLines.length));
    return { nodeId, outcome: "building", build: { state: "running" }, deploy, failure: null, tail: plan.buildLines.slice(0, shown).slice(-1), evidence: plan.evidence };
  }
  if (plan.fails) {
    const message = plan.buildLines.at(-1) ?? "build failed";
    return { nodeId, outcome: "failed", build: { state: "failed", durationMs: plan.build * 60_000 }, deploy: { state: "skipped" },
      failure: { message, containerId: null }, tail: [message], evidence: plan.evidence };
  }
  return { nodeId, outcome: deployOutcome, build: { state: "done", durationMs: plan.build * 60_000 }, deploy,
    failure: null, tail: deploy.state === "done" ? ["healthy"] : [plan.buildLines.at(-1) ?? ""], evidence: plan.evidence };
}

export function useFakeAttempt(): DeploymentAttempt {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const tick = () => setNow(Date.now());
    listeners.add(tick);
    const timer = setInterval(tick, 500);
    return () => { clearInterval(timer); listeners.delete(tick); };
  }, []);
  const t = (now - startedAt) / 1000;
  const plans = Object.values(PLANS[scenario]);
  const buildsEnd = Math.max(...plans.map((plan) => plan.wait + plan.build));
  const failedBuild = plans.some((plan) => plan.fails) && t >= buildsEnd;
  const deployStart = failedBuild ? null : buildsEnd;
  const nodes = NODES.map((node) => nodeView(node.nodeId, t, deployStart, failedBuild));
  const done = deployStart !== null && t >= deployStart + DEPLOY_SECONDS;
  const status = failedBuild ? "failed" : done ? "deployed" : t >= buildsEnd ? "deploying" : "building";
  const deployment = {
    id: FAKE_DEPLOYMENT_ID, environmentId: "", status: failedBuild ? "failed" : done ? "applied" : t >= buildsEnd ? "deploying" : "queued",
    triggerOrigin: { origin: "manual", actorId: "prototype" }, message: "Prototype: Build Order", failureMessage: failedBuild ? "api: build failed" : null,
    inngestRunId: null, coreDeployId: null, deployPreview: null, runtimeProgress: null, sourcePins: {}, buildServiceIds: [],
    canRetry: false, failureCode: null, dispatchRequestedAt: new Date(startedAt), startedAt: new Date(startedAt), finishedAt: null,
    cancellationRequestedAt: null, createdAt: new Date(startedAt), updatedAt: new Date(now), serviceCount: NODES.length,
    projectSlug: "", environmentSlug: "", volumeRemoveAttempts: [],
  } as unknown as EnvironmentDeploymentSummary;
  return {
    deployment,
    nodes: NODES,
    view: { status, deployed: nodes.filter((node) => node.outcome === "deployed").length, changed: NODES.length, nodes },
  };
}

export function evidenceOf(view: DeploymentNodeView): BuilderEvidence | undefined {
  return (view as DeploymentNodeView & { evidence?: BuilderEvidence }).evidence;
}

/** Floating control to replay a scenario. */
export function FakeAttemptReplayBar() {
  const [current, setCurrent] = useState<Scenario>(scenario);
  return (
    <div className="pointer-events-auto fixed bottom-4 left-68 z-50 flex items-center gap-2 rounded-lg border bg-background p-1 shadow-md">
      <span className="px-2 text-muted-foreground text-xs">Prototype replay</span>
      <ToggleGroup
        variant="outline"
        value={[current]}
        onValueChange={(value) => {
          const [next] = value;
          if (!next) return;
          setCurrent(next as Scenario);
          restart(next as Scenario);
        }}
      >
        {(Object.keys(SCENARIO_LABELS) as Scenario[]).map((key) => (
          <ToggleGroupItem key={key} value={key}>{SCENARIO_LABELS[key]}</ToggleGroupItem>
        ))}
      </ToggleGroup>
      <Button size="icon-sm" variant="ghost" aria-label="Replay" onClick={() => restart(current)}>
        <RotateCcwIcon />
      </Button>
    </div>
  );
}

/** Build logs for the fake attempt: the Builder line and skip trail, then the steps so far. */
export function FakeBuildLog({ nodeId, view }: { nodeId: string; view: DeploymentNodeView }) {
  const plan = PLANS[scenario][nodeId];
  const evidence = evidenceOf(view);
  if (!plan || !evidence) return <p className="text-muted-foreground">This Service uses a prebuilt image.</p>;
  const lines = view.build.state === "queued" ? [plan.waitingLine]
    : view.build.state === "running" ? plan.buildLines.slice(0, Math.max(1, plan.buildLines.indexOf(view.tail.at(-1) ?? "") + 1))
    : plan.buildLines;
  return (
    <div className="flex flex-col gap-3 font-mono text-xs">
      <div className="rounded-md border bg-muted/40 px-3 py-2">
        <div>
          {evidence.waiting ? "waiting for " : "built on "}
          <span className="font-medium">{evidence.builder}</span>
          {evidence.waiting ? null : ` · ${evidence.reason}`}
          {evidence.runUrl && !evidence.waiting ? <> · <a className="underline" href={evidence.runUrl} target="_blank" rel="noreferrer">GitHub run ↗</a></> : null}
        </div>
        {evidence.skipped.map((line) => <div key={line} className="text-muted-foreground">skipped: {line}</div>)}
      </div>
      <ol className="flex flex-col gap-1">
        {lines.map((line, index) => (
          <li key={index} className={index === lines.length - 1 && view.failure ? "text-destructive" : undefined}>{line}</li>
        ))}
      </ol>
    </div>
  );
}
