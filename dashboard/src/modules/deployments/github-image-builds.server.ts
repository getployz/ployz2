import "@tanstack/react-start/server-only";
import type { BuildGrantId, BuildReceipt, MachineId, PreparationEvent } from "@ployz/sdk";
import { and, eq, isNotNull, isNull } from "drizzle-orm";
import { Effect, Schema } from "effect";
import { cancelGithubRun, checkGithubBuildWorkflow, dispatchGithubBuildWorkflow, githubRunCompleted } from "#/modules/github/github-build.server";
import { verifyGithubOidcToken } from "#/modules/github/github-oidc.server";
import { buildFingerprints, ployzVersion } from "#/modules/runtime/ployz.server";
import { AppConfig } from "#/server/config.server";
import { Database } from "#/server/database.server";
import { Conflict, Forbidden, NotFound, Unauthorized, Validation } from "#/server/public-error";
import { persistBuildLog } from "./deployment-events.server";
import {
  settleImageBuildResult, skipImageBuilder, START_WITHIN_MINUTES,
  type ImageBuildAttempt, type ImageBuildResult, type ImageBuildTarget,
} from "./image-builds.server";
import { preparationProgressCollector, type BuildOutputWrite, type BuildStepWrite } from "./preparation-progress";
import { compileRuntimeIntent, connectedRuntime } from "./runtime-activities.server";
import { loadDeploymentContext } from "./runtime-hydration.repository.server";
import type { DeploymentContext } from "./runtime-repository.contract";
import { pinSourceCommit } from "./runtime-sources.server";
import { environmentDeploymentImageBuild as imageBuild } from "./tables";

/**
 * GitHub as a Builder. Cloud dispatches the repository's build workflow, the runner checks in once
 * with its OIDC token for the build's grant and secrets, posts its Build Steps, and when the run
 * completes Cloud ends the grant and writes the receipt from the digest the Machine received.
 *
 *   dispatch ──▶ check-in (once) ──▶ steps ──▶ workflow_run completed ──▶ end grant ──▶ receipt
 *
 * Check-in is the build starting. Until then GitHub can still be skipped: at once when it can't take
 * the build, at the "start within" limit, or when the run ends first.
 */

/**
 * How long a run may build after it checked in before Cloud cancels it. The Build Grant is minted at
 * check-in and must outlive this: the daemon's `GRANT_LIFETIME` (3h, ployzd `management/build_grant.rs`).
 */
export const GITHUB_RUN_BUDGET_MS = 2 * 60 * 60_000;
/** How often Cloud looks at a dispatched run on GitHub between Workflow run webhooks. */
export const GITHUB_CHECK_INTERVAL = "10m";

export type GithubBuildStart = ImageBuildAttempt | { kind: "dispatched"; runId: number };

type Snapshot = DeploymentContext["snapshots"][number];
const installedSource = (snapshot: Snapshot | undefined) => {
  const source = snapshot?.config.source;
  return source?.type === "git" && source.access.type === "github-installation"
    ? { ...source, installationId: source.access.installationId } : null;
};

/** The same one-Service deployment a server build gets, so fingerprints and platforms match deploy's. */
const oneServiceDeployment = (context: DeploymentContext, serviceId: string) => compileRuntimeIntent(context).pipe(
  Effect.map((intent) => ({ ...intent, snapshots: intent.snapshots.filter((candidate) => candidate.serviceId === serviceId), dependencies: {} })),
);

/** The GitHub-hosted runner that builds each platform natively. */
const GITHUB_RUNNERS = new Map([["linux/amd64", "ubuntu-latest"], ["linux/arm64", "ubuntu-24.04-arm"]]);

const skipGithub = (build: ImageBuildTarget, reason: string) =>
  skipImageBuilder(build.id, reason).pipe(Effect.as<GithubBuildStart>({ kind: "skipped", reason }));

/** The row's status as the walk reports it, once something else settled it. */
const currentResult = Effect.fn("Deployments.currentImageBuildResult")(function* (build: ImageBuildTarget) {
  const { drizzle } = yield* Database;
  const [row] = yield* drizzle.select({ status: imageBuild.status }).from(imageBuild).where(eq(imageBuild.id, build.id)).limit(1);
  return { imageBuildId: build.id, image: build.image, status: row?.status ?? "failed" } satisfies ImageBuildResult;
});

/**
 * Dispatches an Image Build to GitHub Actions on the runner for its one platform. GitHub is skipped
 * at once, with the reason on the Image Build, when it can't take the build: the repository isn't
 * reachable through the GitHub App or lacks permission, has no workflow, needs several platforms,
 * or the dispatch fails.
 */
export const startGithubImageBuild = Effect.fn("Deployments.startGithubImageBuild")(function* (build: ImageBuildTarget) {
  const context = yield* loadDeploymentContext(build.deploymentId);
  const snapshot = context?.snapshots.find((candidate) => candidate.serviceId === build.serviceId);
  const source = installedSource(snapshot);
  if (!context || !snapshot || !source) return yield* skipGithub(build, "GitHub: the repository isn't connected through the GitHub App");
  const workflow = yield* checkGithubBuildWorkflow(source.installationId, source.repositoryId);
  const repository = workflow.fullName ?? source.repository;
  if (workflow.readiness === "no_permission") return yield* skipGithub(build, `GitHub: no permission in ${repository}`);
  if (workflow.readiness !== "ready" || !workflow.fullName || !workflow.defaultBranch) return yield* skipGithub(build, `GitHub: no workflow in ${repository}`);
  const deployment = yield* oneServiceDeployment(context, build.serviceId);
  const platforms = yield* connectedRuntime(context.organization.id).pipe(Effect.flatMap((sdk) => sdk.buildPlatforms(deployment)), Effect.scoped);
  if (platforms.length > 1) return yield* skipGithub(build, `GitHub: needs ${platforms.map((platform) => platform.replace(/^linux\//, "")).join("+")}`);
  // With no visible placement, deploy's coverage check decides, as it does after a server build.
  const platform = platforms[0] ?? "linux/amd64";
  const runner = GITHUB_RUNNERS.get(platform);
  if (!runner) return yield* skipGithub(build, `GitHub: no runner for ${platform}`);
  // The check-in hands the runner this pinned commit.
  yield* pinSourceCommit(context, snapshot, source);
  const config = yield* AppConfig;
  const run = yield* dispatchGithubBuildWorkflow({
    installationId: source.installationId, fullName: workflow.fullName, defaultBranch: workflow.defaultBranch,
    inputs: { build: build.id, cloud: config.app.url.origin, ployz_version: ployzVersion(), runner },
  });
  const { drizzle } = yield* Database;
  const [recorded] = yield* drizzle.update(imageBuild).set({ builder: "github", githubRunId: run.runId, githubRunUrl: run.runUrl, githubWorkflowRef: run.workflowRef, updatedAt: new Date() })
    .where(and(eq(imageBuild.id, build.id), eq(imageBuild.status, "building"))).returning({ id: imageBuild.id });
  if (!recorded) {
    // Settled (cancelled) while dispatching: the run must not build.
    yield* cancelGithubRun({ installationId: source.installationId, fullName: workflow.fullName, runId: run.runId }).pipe(Effect.ignore);
    return { kind: "settled", result: yield* currentResult(build) } satisfies GithubBuildStart;
  }
  return { kind: "dispatched", runId: run.runId } satisfies GithubBuildStart;
}, (effect, build) => effect.pipe(
  // A GitHub or SDK failure before dispatch is GitHub being unusable, not the build failing.
  Effect.catch((error) => skipGithub(build, `GitHub: could not start the build (${error.message})`)),
));

/**
 * GitHub's "start within" limit passed. A run that checked in has started and keeps the build;
 * otherwise GitHub is skipped and its run cancelled. Check-in and this skip update the same row
 * under exclusive conditions, so exactly one wins.
 */
export const withdrawGithubImageBuild = Effect.fn("Deployments.withdrawGithubImageBuild")(function* (build: ImageBuildTarget) {
  const { drizzle } = yield* Database;
  const [row] = yield* drizzle.select().from(imageBuild).where(eq(imageBuild.id, build.id)).limit(1);
  const reason = `GitHub: no runner in ${START_WITHIN_MINUTES} min`;
  if (row?.status === "building" && (yield* skipImageBuilder(build.id, reason))) {
    yield* cancelGithubBuildRun(row);
    return { kind: "skipped", reason } satisfies GithubBuildCheck;
  }
  const result = yield* currentResult(build);
  return result.status === "building" ? { kind: "waiting" as const } : { kind: "settled" as const, result };
});

/** What one look at a dispatched GitHub build finds: it settled, GitHub was skipped, or it goes on. */
export type GithubBuildCheck = ImageBuildAttempt | { kind: "waiting" };

/**
 * One look at a dispatched build, after the Workflow run webhook reported its run `ended` or a wait
 * timed out. A timeout also asks GitHub whether the run completed, which catches a completion that
 * landed between two waits. `startLimit`: this Builder isn't last and its "start within" passed.
 * The last Builder waits for its run to start without a limit; once started, a run gets
 * GITHUB_RUN_BUDGET_MS.
 */
export const checkGithubImageBuild = Effect.fn("Deployments.checkGithubImageBuild")(function* (
  build: ImageBuildTarget, seen: { ended: boolean; startLimit: boolean },
) {
  const { drizzle } = yield* Database;
  const [row] = yield* drizzle.select().from(imageBuild).where(eq(imageBuild.id, build.id)).limit(1);
  if (row?.status !== "building") return { kind: "settled", result: yield* currentResult(build) } satisfies GithubBuildCheck;
  if (seen.ended || (yield* githubRunEnded(row))) return yield* finishGithubImageBuild(build, false);
  if (row.checkedInAt === null) return seen.startLimit ? yield* withdrawGithubImageBuild(build) : { kind: "waiting" } satisfies GithubBuildCheck;
  if (Date.now() - row.checkedInAt.getTime() > GITHUB_RUN_BUDGET_MS) return yield* finishGithubImageBuild(build, true);
  return { kind: "waiting" } satisfies GithubBuildCheck;
});

/** Whether GitHub says the build's run completed; unknown (GitHub unreachable) reads as still running. */
const githubRunEnded = (row: GithubRunRow) => Effect.gen(function* () {
  const context = yield* loadDeploymentContext(row.deploymentId);
  const source = installedSource(context?.snapshots.find((snapshot) => snapshot.serviceId === row.serviceId));
  const fullName = row.githubWorkflowRef?.split("/.github/")[0];
  if (!source || !fullName || row.githubRunId === null) return false;
  return yield* githubRunCompleted({ installationId: source.installationId, fullName, runId: row.githubRunId });
}).pipe(Effect.orElseSucceed(() => false));

const bearer = (request: Request) => {
  const match = /^Bearer (\S+)$/.exec(request.headers.get("authorization") ?? "");
  return match?.[1] ?? null;
};

/** The runner's OIDC token must name this build's repository, workflow on the default branch, and run. */
const authorizeRunner = Effect.fn("Deployments.authorizeGithubRunner")(function* (request: Request, imageBuildId: string) {
  const token = bearer(request);
  if (!token) return yield* new Unauthorized();
  const config = yield* AppConfig;
  const claims = yield* verifyGithubOidcToken(token, config.app.url.origin).pipe(Effect.mapError(() => new Unauthorized()));
  const { drizzle } = yield* Database;
  const [row] = yield* drizzle.select().from(imageBuild).where(eq(imageBuild.id, imageBuildId)).limit(1);
  if (!row || row.builder !== "github" || row.githubRunId === null) return yield* new NotFound({ message: "No GitHub build has this id." });
  const context = yield* loadDeploymentContext(row.deploymentId);
  const source = installedSource(context?.snapshots.find((snapshot) => snapshot.serviceId === row.serviceId));
  if (!context || !source) return yield* new NotFound({ message: "No GitHub build has this id." });
  if (claims.repository_id !== String(source.repositoryId)) return yield* new Forbidden({ message: "The token is for another repository." });
  if (claims.job_workflow_ref !== row.githubWorkflowRef) return yield* new Forbidden({ message: "The token is for another workflow or branch." });
  if (claims.run_id !== String(row.githubRunId)) return yield* new Forbidden({ message: "The token is for another run." });
  if (claims.event_name !== "workflow_dispatch") return yield* new Forbidden({ message: "The run was not dispatched by Ployz." });
  return { row, context, githubRunId: row.githubRunId };
});

/**
 * The runner's one check-in: the build starts. Accepted once, while GitHub still holds the build.
 * Mints a Build Grant on the Machine Cloud deploys through and returns it with the commit, the
 * expected fingerprint, and the frozen deployment whose `resolvedEnv` carries the build secrets.
 * Nothing secret is ever a workflow input.
 */
export const checkInGithubBuild = Effect.fn("Deployments.checkInGithubBuild")(function* (request: Request, imageBuildId: string) {
  const { row, context, githubRunId } = yield* authorizeRunner(request, imageBuildId);
  const { drizzle } = yield* Database;
  // The run must still hold the build: a skip at the start limit clears it in the same row.
  const [claimed] = yield* drizzle.update(imageBuild).set({ checkedInAt: new Date(), updatedAt: new Date() })
    .where(and(eq(imageBuild.id, row.id), eq(imageBuild.status, "building"), isNull(imageBuild.checkedInAt), eq(imageBuild.githubRunId, githubRunId)))
    .returning({ id: imageBuild.id });
  if (!claimed) return yield* new Conflict({ message: "This build already checked in or is no longer wanted." });
  const snapshot = context.snapshots.find((candidate) => candidate.serviceId === row.serviceId);
  const source = snapshot?.config.source;
  if (!snapshot || source?.type !== "git") return yield* new NotFound({ message: "No GitHub build has this id." });
  const commit = yield* pinSourceCommit(context, snapshot, source);
  const deployment = yield* oneServiceDeployment(context, row.serviceId);
  const fingerprint = buildFingerprints({ deployment, source_commits: { [row.image]: commit } })[row.image];
  if (!fingerprint) return yield* new Validation({ message: "The build has no fingerprint." });
  const sdk = yield* connectedRuntime(context.organization.id);
  const minted = yield* sdk.mintBuildGrant(`ployz-build/${row.image}`);
  const machine = yield* sdk.inspect();
  yield* drizzle.update(imageBuild).set({ grantId: minted.id, machineId: machine.id, fingerprint, updatedAt: new Date() })
    .where(eq(imageBuild.id, row.id));
  return { grant: minted.grant, commit, fingerprint, deployment };
}, Effect.scoped);

const buildStepSchema = Schema.Struct({
  id: Schema.String, name: Schema.String, started: Schema.NullOr(Schema.String), completed: Schema.NullOr(Schema.String),
  cached: Schema.Boolean, error: Schema.NullOr(Schema.String),
});
/** The runner's `ployz build --events` lines, and the platforms it built. */
const stepsReportSchema = Schema.Struct({
  events: Schema.Array(Schema.Struct({
    at: Schema.Number,
    event: Schema.Struct({ Build: Schema.Union([
      Schema.Struct({ Stage: Schema.String }),
      Schema.Struct({ Output: Schema.Array(Schema.Number) }),
      Schema.Struct({ Step: buildStepSchema }),
      Schema.Struct({ StepOutput: Schema.Struct({ step: Schema.String, stderr: Schema.Boolean, text: Schema.String }) }),
      Schema.Struct({ Timing: Schema.Unknown }),
      Schema.Struct({ Target: Schema.Struct({ name: Schema.String, outcome: Schema.Unknown }) }),
    ]) }),
  })),
  platforms: Schema.Array(Schema.String),
});

const MAX_STEPS_REPORT_BYTES = 16 * 1024 * 1024;

/**
 * The runner's Build Steps, filed under the Image Build like a server build's, so they show in the
 * same log. Accepted once, after check-in. An empty `platforms` means the build failed.
 */
export const recordGithubBuildSteps = Effect.fn("Deployments.recordGithubBuildSteps")(function* (request: Request, imageBuildId: string, text: string) {
  const { row } = yield* authorizeRunner(request, imageBuildId);
  if (text.length > MAX_STEPS_REPORT_BYTES) return yield* new Validation({ message: "The report is too large." });
  const report = yield* Schema.decodeUnknownEffect(Schema.fromJsonString(stepsReportSchema))(text)
    .pipe(Effect.mapError(() => new Validation({ message: "The report is not Build Steps." })));
  const { drizzle } = yield* Database;
  const [claimed] = yield* drizzle.update(imageBuild).set({ platforms: [...report.platforms], updatedAt: new Date() })
    .where(and(eq(imageBuild.id, row.id), isNotNull(imageBuild.checkedInAt), isNull(imageBuild.platforms)))
    .returning({ id: imageBuild.id });
  if (!claimed) return yield* new Conflict({ message: "This build has not checked in or already reported." });
  let at = new Date();
  const collector = preparationProgressCollector(() => at);
  const steps: BuildStepWrite[] = [];
  const output: BuildOutputWrite[] = [];
  for (const line of report.events) {
    at = new Date(line.at);
    // SAFETY: the schema above admits only `Build` progress, a subset of PreparationEvent.
    const writes = collector.event(line.event as PreparationEvent);
    steps.push(...writes.steps);
    output.push(...writes.output);
  }
  steps.push(...collector.finish(report.platforms.length ? null : "The build failed on GitHub; see its run."));
  yield* persistBuildLog(row.deploymentId, { steps, output }, row.image);
  return { accepted: true };
});

/**
 * Settles a GitHub build once its run ended, or ran out of budget (`timedOut`: its run is cancelled
 * first): ends the grant and writes the receipt from the digest the Machine verified. A run that
 * ended before it checked in never started, so GitHub is skipped. A started run that pushed nothing failed.
 */
export const finishGithubImageBuild = Effect.fn("Deployments.finishGithubImageBuild")(function* (build: ImageBuildTarget, timedOut: boolean) {
  const { drizzle } = yield* Database;
  const load = Effect.fn(function* () {
    const [loaded] = yield* drizzle.select().from(imageBuild).where(eq(imageBuild.id, build.id)).limit(1);
    return loaded;
  });
  const found = yield* load();
  if (found?.status !== "building") return { kind: "settled", result: yield* currentResult(build) } satisfies ImageBuildAttempt;
  if (found.checkedInAt === null) {
    const reason = "GitHub: the run ended before it started";
    if (yield* skipImageBuilder(build.id, reason)) return { kind: "skipped", reason } satisfies ImageBuildAttempt;
  }
  // It started, perhaps just now: re-read what check-in recorded.
  const row = found.checkedInAt === null ? yield* load() : found;
  if (row?.status !== "building") return { kind: "settled", result: yield* currentResult(build) } satisfies ImageBuildAttempt;
  if (timedOut) yield* cancelGithubBuildRun(row);
  const failed = (message: string) => settleImageBuildResult(build, { status: "failed", message, machineId: row.machineId })
    .pipe(Effect.map((result): ImageBuildAttempt => ({ kind: "settled", result })));
  if (!row.grantId || !row.machineId || !row.fingerprint) return yield* failed("GitHub: the run ended before it received its grant.");
  const pushed = yield* endGrant(row.organizationId, row.machineId, row.grantId).pipe(
    Effect.map((ended) => ended.pushed ?? null),
    Effect.orElseSucceed(() => null),
  );
  if (!pushed || !row.platforms?.length) return yield* failed(timedOut ? "GitHub: the run didn't finish within 2 hours." : "GitHub: the run pushed no image.");
  // SAFETY: machineId was read from the Machine's own inspect at check-in.
  const machineId = row.machineId as MachineId;
  const receipt: BuildReceipt = {
    fingerprint: row.fingerprint, machine_id: machineId,
    image: { reference: pushed, tags: [`ployz-build/${row.image}:ployz-sha256-${pushed.replace(/^sha256:/, "")}`], platforms: [...row.platforms], location: "build-grant" },
  };
  return { kind: "settled", result: yield* settleImageBuildResult(build, { status: "built", receipt }) } satisfies ImageBuildAttempt;
});

type GithubRunRow = Pick<typeof imageBuild.$inferSelect, "deploymentId" | "serviceId" | "organizationId" | "githubRunId" | "githubWorkflowRef" | "grantId" | "machineId">;

/** Best effort: cancel the run and end its grant, so the grant refuses any further push. */
const cancelGithubBuildRun = (row: GithubRunRow) => Effect.gen(function* () {
  const context = yield* loadDeploymentContext(row.deploymentId);
  const source = installedSource(context?.snapshots.find((snapshot) => snapshot.serviceId === row.serviceId));
  const fullName = row.githubWorkflowRef?.split("/.github/")[0];
  if (source && fullName && row.githubRunId !== null) {
    yield* cancelGithubRun({ installationId: source.installationId, fullName, runId: row.githubRunId }).pipe(Effect.ignore);
  }
  if (row.grantId && row.machineId) yield* endGrant(row.organizationId, row.machineId, row.grantId).pipe(Effect.ignore);
}).pipe(Effect.catch((error) => Effect.logWarning("Could not cancel a GitHub build run.", error)));

/** Idempotent. Only the Machine that minted the grant knows it. */
const endGrant = (organizationId: string, machineId: string, grantId: string) =>
  // SAFETY: both ids were read from that Machine at check-in: its inspect and its mint.
  connectedRuntime(organizationId, machineId as MachineId).pipe(
    Effect.flatMap((sdk) => sdk.endBuildGrant(grantId as BuildGrantId)),
    Effect.scoped,
  );

/** An ended attempt's GitHub builds: cancel each run and end its grant. Idempotent. */
export const cancelGithubImageBuilds = Effect.fn("Deployments.cancelGithubImageBuilds")(function* (inngestRunId: string) {
  const { drizzle } = yield* Database;
  const rows = yield* drizzle.select().from(imageBuild)
    .where(and(eq(imageBuild.inngestRunId, inngestRunId), eq(imageBuild.builder, "github"), isNotNull(imageBuild.githubRunId)));
  yield* Effect.forEach(rows, cancelGithubBuildRun, { concurrency: 4, discard: true });
  return rows.length;
});
