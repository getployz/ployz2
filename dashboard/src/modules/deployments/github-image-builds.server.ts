import "@tanstack/react-start/server-only";
import type { BuildGrantId, BuildReceipt, MachineId, PreparationEvent } from "@ployz/sdk";
import { and, eq, isNotNull, isNull } from "drizzle-orm";
import { Effect, Schema } from "effect";
import { cancelGithubRun, checkGithubBuildWorkflow, dispatchGithubBuildWorkflow } from "#/modules/github/github-build.server";
import { verifyGithubOidcToken } from "#/modules/github/github-oidc.server";
import { buildFingerprints, ployzVersion } from "#/modules/runtime/ployz.server";
import { AppConfig } from "#/server/config.server";
import { Database } from "#/server/database.server";
import { Conflict, Forbidden, NotFound, Unauthorized, Validation } from "#/server/public-error";
import { loadBuildOrder } from "./build-order.server";
import { persistBuildLog } from "./deployment-events.server";
import { settleImageBuild, type ImageBuildOutcome, type ImageBuildTarget } from "./image-builds.server";
import { preparationProgressCollector, type BuildOutputWrite, type BuildStepWrite } from "./preparation-progress";
import { compileRuntimeIntent, connectedRuntime } from "./runtime-activities.server";
import { loadDeploymentContext } from "./runtime-hydration.repository.server";
import type { DeploymentContext } from "./runtime-repository.contract";
import { pinSourceCommit } from "./runtime-sources.server";
import { environmentDeploymentImageBuild as imageBuild, type ImageBuildStatus } from "./tables";

/**
 * GitHub as a Builder. Cloud dispatches the repository's build workflow, the runner checks in once
 * with its OIDC token for the build's grant and secrets, posts its Build Steps, and when the run
 * completes Cloud ends the grant and writes the receipt from the digest the Machine received.
 *
 *   dispatch ──▶ check-in (once) ──▶ steps ──▶ workflow_run completed ──▶ end grant ──▶ receipt
 */

/** How long a dispatched run may take, queueing included, before Cloud cancels it. */
export const GITHUB_RUN_TIMEOUT = "2h";

export type ImageBuildResult = { imageBuildId: string; image: string; status: ImageBuildStatus };
export type GithubBuildStart =
  | { kind: "servers" }
  | { kind: "dispatched"; runId: number }
  | { kind: "settled"; result: ImageBuildResult };

type Snapshot = DeploymentContext["snapshots"][number];
const installedSource = (snapshot: Snapshot | undefined) => {
  const source = snapshot?.config.source;
  return source?.type === "git" && source.access.type === "github-installation"
    ? { ...source, installationId: source.access.installationId } : null;
};

const settled = (build: ImageBuildTarget, outcome: ImageBuildOutcome) =>
  settleImageBuild(build.id, outcome).pipe(Effect.map((status): ImageBuildResult => ({ imageBuildId: build.id, image: build.image, status })));

/**
 * Starts an Image Build on GitHub when the Organization's Build Order says so. With GitHub unusable,
 * "GitHub only" fails the build and "GitHub, then your servers" builds on the servers.
 */
export const startGithubImageBuild = Effect.fn("Deployments.startGithubImageBuild")(function* (build: ImageBuildTarget) {
  const context = yield* loadDeploymentContext(build.deploymentId);
  if (!context) return { kind: "servers" } satisfies GithubBuildStart;
  const order = yield* loadBuildOrder(context.organization.id);
  if (order === "servers-only") return { kind: "servers" } satisfies GithubBuildStart;
  // ponytail: no skip trail yet; #1084 records why GitHub was skipped.
  const unusable = (message: string) => order === "github-only"
    ? settled(build, { status: "failed", message, machineId: null }).pipe(Effect.map((result): GithubBuildStart => ({ kind: "settled", result })))
    : Effect.succeed<GithubBuildStart>({ kind: "servers" });
  const snapshot = context.snapshots.find((candidate) => candidate.serviceId === build.serviceId);
  const source = installedSource(snapshot);
  if (!snapshot || !source) return yield* unusable("GitHub: the repository isn't connected through the GitHub App.");
  const workflow = yield* checkGithubBuildWorkflow(source.installationId, source.repositoryId);
  if (workflow.readiness !== "ready" || !workflow.fullName || !workflow.defaultBranch) {
    return yield* unusable(`GitHub: no workflow in ${workflow.fullName ?? source.repository}.`);
  }
  // The check-in hands the runner this pinned commit.
  yield* pinSourceCommit(context, snapshot, source);
  const config = yield* AppConfig;
  const run = yield* dispatchGithubBuildWorkflow({
    installationId: source.installationId, fullName: workflow.fullName, defaultBranch: workflow.defaultBranch,
    // ponytail: amd64 runners only; an arm64-only cluster's receipt won't cover its Servers. #1084 picks by platform.
    inputs: { build: build.id, cloud: config.app.url.origin, ployz_version: ployzVersion(), runner: "ubuntu-latest" },
  });
  const { drizzle } = yield* Database;
  yield* drizzle.update(imageBuild).set({ builder: "github", githubRunId: run.runId, githubRunUrl: run.runUrl, githubWorkflowRef: run.workflowRef, updatedAt: new Date() })
    .where(and(eq(imageBuild.id, build.id), eq(imageBuild.status, "building")));
  return { kind: "dispatched", runId: run.runId } satisfies GithubBuildStart;
}, (effect, build) => effect.pipe(
  // A GitHub or SDK failure before dispatch is GitHub being unusable, not the build failing.
  Effect.catch((error) => Effect.gen(function* () {
    const context = yield* loadDeploymentContext(build.deploymentId);
    const order = context ? yield* loadBuildOrder(context.organization.id) : "servers-only";
    if (order !== "github-only") return { kind: "servers" } satisfies GithubBuildStart;
    const message = `GitHub: could not start the build (${error.message}).`;
    return { kind: "settled", result: yield* settled(build, { status: "failed", message, machineId: null }) } satisfies GithubBuildStart;
  })),
));

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
  return { row, context };
});

/**
 * The runner's one check-in. Accepted once, while the build is still wanted. Mints a Build Grant on
 * the Machine Cloud deploys through and returns it with the commit, the expected fingerprint, and the
 * frozen deployment whose `resolvedEnv` carries the build secrets. Nothing secret is ever a workflow input.
 */
export const checkInGithubBuild = Effect.fn("Deployments.checkInGithubBuild")(function* (request: Request, imageBuildId: string) {
  const { row, context } = yield* authorizeRunner(request, imageBuildId);
  const { drizzle } = yield* Database;
  const [claimed] = yield* drizzle.update(imageBuild).set({ checkedInAt: new Date(), updatedAt: new Date() })
    .where(and(eq(imageBuild.id, row.id), eq(imageBuild.status, "building"), isNull(imageBuild.checkedInAt)))
    .returning({ id: imageBuild.id });
  if (!claimed) return yield* new Conflict({ message: "This build already checked in or is no longer wanted." });
  const snapshot = context.snapshots.find((candidate) => candidate.serviceId === row.serviceId);
  const source = snapshot?.config.source;
  if (!snapshot || source?.type !== "git") return yield* new NotFound({ message: "No GitHub build has this id." });
  const commit = yield* pinSourceCommit(context, snapshot, source);
  const intent = yield* compileRuntimeIntent(context);
  // The same one-Service deployment a server build gets, so the fingerprint matches deploy's.
  const deployment = { ...intent, snapshots: intent.snapshots.filter((candidate) => candidate.serviceId === row.serviceId), dependencies: {} };
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
 * Settles a GitHub build once its run ended (or ran out of time): ends the grant and writes the
 * receipt from the digest the Machine verified. A run that never checked in, or pushed nothing, failed.
 */
export const finishGithubImageBuild = Effect.fn("Deployments.finishGithubImageBuild")(function* (build: ImageBuildTarget, timedOut: boolean) {
  const { drizzle } = yield* Database;
  const [row] = yield* drizzle.select().from(imageBuild).where(eq(imageBuild.id, build.id)).limit(1);
  if (!row) return { imageBuildId: build.id, image: build.image, status: "failed" } satisfies ImageBuildResult;
  if (timedOut) yield* cancelGithubBuildRun(row);
  if (row.status !== "building") return { imageBuildId: build.id, image: build.image, status: row.status } satisfies ImageBuildResult;
  const failed = (message: string) => settled(build, { status: "failed", message, machineId: row.machineId });
  if (!row.grantId || !row.machineId || !row.fingerprint) {
    return yield* failed(timedOut ? "GitHub: the run didn't start in time." : "GitHub: the run ended before it checked in.");
  }
  const pushed = yield* endGrant(row.organizationId, row.machineId, row.grantId).pipe(
    Effect.map((ended) => ended.pushed ?? null),
    Effect.orElseSucceed(() => null),
  );
  if (!pushed || !row.platforms?.length) return yield* failed("GitHub: the run pushed no image.");
  // SAFETY: machineId was read from the Machine's own inspect at check-in.
  const machineId = row.machineId as MachineId;
  const receipt: BuildReceipt = {
    fingerprint: row.fingerprint, machine_id: machineId,
    image: { reference: pushed, tags: [`ployz-build/${row.image}:ployz-sha256-${pushed.replace(/^sha256:/, "")}`], platforms: [...row.platforms], location: "build-grant" },
  };
  return yield* settled(build, { status: "built", receipt });
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

/** A cancelled attempt's GitHub builds: cancel each run and end its grant. Idempotent. */
export const cancelGithubImageBuilds = Effect.fn("Deployments.cancelGithubImageBuilds")(function* (inngestRunId: string) {
  const { drizzle } = yield* Database;
  const rows = yield* drizzle.select().from(imageBuild)
    .where(and(eq(imageBuild.inngestRunId, inngestRunId), eq(imageBuild.builder, "github"), isNotNull(imageBuild.githubRunId)));
  yield* Effect.forEach(rows, cancelGithubBuildRun, { concurrency: 4, discard: true });
  return rows.length;
});
