import "@tanstack/react-start/server-only";
import type { BuildReceipt, BuildReceipts, MachineId } from "@ployz/sdk";
import { and, desc, eq, inArray, isNull, sql } from "drizzle-orm";
import { Effect, Option, Schema } from "effect";
import { organizationIdForDeployment } from "#/db/scope-values.server";
import { rustMachineIdSchema } from "#/modules/machines/enrollment";
import { Database } from "#/server/database.server";
import { SecretEncryption } from "#/utils/encrypted-secret.server";
import { ACTIVE_ENVIRONMENT_DEPLOYMENT_STATUSES } from "./runtime-contract";
import type { DeploymentContext } from "./runtime-repository.contract";
import { environmentDeployment, environmentDeploymentImageBuild, type ImageBuildStatus, type ServerChoice } from "./tables";

const buildReceiptSchema = Schema.Struct({
  fingerprint: Schema.String.check(Schema.isPattern(/^[0-9a-f]{64}$/)),
  machine_id: Schema.declare<MachineId>((value): value is MachineId =>
    Schema.is(rustMachineIdSchema)(value)),
  image: Schema.Struct({
    reference: Schema.String.check(Schema.isPattern(/^sha256:[0-9a-f]{64}$/)),
    tags: Schema.mutable(Schema.Array(Schema.String)),
    platforms: Schema.mutable(Schema.Array(Schema.String)),
    location: Schema.String,
  }),
});

/**
 * What one Image Build builds: a Git Service of the attempt, named by its image. `buildIndex` is its
 * position among the attempt's builds, so the Engine spreads them across Servers.
 */
export type ImageBuildTarget = { id: string; deploymentId: string; serviceId: string; image: string; buildIndex: number };

export type ImageBuildOutcome =
  | { status: "built"; receipt: BuildReceipt }
  | { status: "failed"; message: string; machineId: string | null }
  | { status: "cancelled" };

export type ImageBuildResult = { imageBuildId: string; image: string; status: ImageBuildStatus };
/** One Builder's go at an Image Build: it settled there, or the Builder didn't take it and the walk moves on. */
export type ImageBuildAttempt = { kind: "settled"; result: ImageBuildResult } | { kind: "skipped"; reason: string };

/** How long a Builder that isn't last in the walk has to start a build before the next Builder gets it. */
export const START_WITHIN_MINUTES = 3;
export const START_WITHIN_MS = START_WITHIN_MINUTES * 60_000;

/** The attempt's Git Services each get one Image Build. */
export const imageBuildServices = (context: Pick<DeploymentContext, "snapshots">) =>
  context.snapshots.filter((snapshot) => snapshot.config.source.type === "git");

/** Admission fan-out: one building row per Git Service, owned by the attempt's run. Idempotent. */
export const startImageBuilds = Effect.fn("Deployments.startImageBuilds")(function* (context: DeploymentContext, runId: string) {
  const { drizzle } = yield* Database;
  const deploymentId = context.deployment.id;
  const services = imageBuildServices(context);
  const [owned] = yield* drizzle.select({ id: environmentDeployment.id }).from(environmentDeployment).where(and(
    eq(environmentDeployment.id, deploymentId), eq(environmentDeployment.inngestRunId, runId),
    inArray(environmentDeployment.status, [...ACTIVE_ENVIRONMENT_DEPLOYMENT_STATUSES]),
  )).limit(1);
  if (!owned || !services.length) return [];
  yield* drizzle.insert(environmentDeploymentImageBuild).values(services.map((snapshot) => ({
    organizationId: organizationIdForDeployment(deploymentId), deploymentId, inngestRunId: runId,
    serviceId: snapshot.serviceId, image: snapshot.config.privateDns,
  }))).onConflictDoNothing();
  const rows = yield* drizzle.select({
    id: environmentDeploymentImageBuild.id, deploymentId: environmentDeploymentImageBuild.deploymentId,
    serviceId: environmentDeploymentImageBuild.serviceId, image: environmentDeploymentImageBuild.image,
  }).from(environmentDeploymentImageBuild).where(eq(environmentDeploymentImageBuild.deploymentId, deploymentId))
    .orderBy(environmentDeploymentImageBuild.serviceId);
  return rows.map((row, buildIndex): ImageBuildTarget => ({ ...row, buildIndex }));
});

/** Records the Server the Engine chose while the row still builds, so the choice shows during the build. */
export const recordServerChoice = Effect.fn("Deployments.recordServerChoice")(function* (imageBuildId: string, machineId: string, serverChoice: ServerChoice) {
  const { drizzle } = yield* Database;
  yield* drizzle.update(environmentDeploymentImageBuild).set({ machineId, serverChoice, updatedAt: new Date() })
    .where(and(eq(environmentDeploymentImageBuild.id, imageBuildId), eq(environmentDeploymentImageBuild.status, "building")));
});

/** Settles a building row once. Receipts are private evidence and stored encrypted. */
export const settleImageBuild = Effect.fn("Deployments.settleImageBuild")(function* (imageBuildId: string, outcome: ImageBuildOutcome) {
  const { drizzle } = yield* Database;
  const encryption = yield* SecretEncryption;
  const now = new Date();
  const patch = outcome.status === "built"
    ? yield* Schema.decodeUnknownEffect(buildReceiptSchema)(outcome.receipt, { onExcessProperty: "error" }).pipe(
        Effect.map((receipt) => ({ status: "built" as const, machineId: receipt.machine_id, encryptedReceipt: encryption.encrypt(JSON.stringify(receipt)) })),
        Effect.orElseSucceed(() => ({ status: "failed" as const, failureMessage: "Completed build evidence is invalid." })),
      )
    : outcome.status === "failed" ? { status: "failed" as const, machineId: outcome.machineId, failureMessage: outcome.message }
    : { status: "cancelled" as const };
  yield* drizzle.update(environmentDeploymentImageBuild).set({ ...patch, finishedAt: now, updatedAt: now })
    .where(and(eq(environmentDeploymentImageBuild.id, imageBuildId), eq(environmentDeploymentImageBuild.status, "building")));
  return patch.status;
});

export const settleImageBuildResult = (build: Pick<ImageBuildTarget, "id" | "image">, outcome: ImageBuildOutcome) =>
  settleImageBuild(build.id, outcome).pipe(Effect.map((status): ImageBuildResult => ({ imageBuildId: build.id, image: build.image, status })));

/**
 * Adds a Builder that didn't take the build to its skip trail and clears what that Builder left, so
 * the next one starts clean. Refused once the build started on GitHub (checked in) or settled: a
 * build that started never moves.
 */
export const skipImageBuilder = Effect.fn("Deployments.skipImageBuilder")(function* (imageBuildId: string, reason: string) {
  const { drizzle } = yield* Database;
  const table = environmentDeploymentImageBuild;
  const [skipped] = yield* drizzle.update(table).set({
    skips: sql`array_append(${table.skips}, ${reason})`, builder: "server", machineId: null, serverChoice: null,
    githubRunId: null, githubRunUrl: null, githubWorkflowRef: null, updatedAt: new Date(),
  }).where(and(eq(table.id, imageBuildId), eq(table.status, "building"), isNull(table.checkedInAt))).returning({ id: table.id });
  return skipped !== undefined;
});

/** An Image Build is wanted while it builds and its attempt is active and not being cancelled. */
export const imageBuildWanted = Effect.fn("Deployments.imageBuildWanted")(function* (imageBuildId: string) {
  const { drizzle } = yield* Database;
  const [row] = yield* drizzle.select({
    build: environmentDeploymentImageBuild.status, attempt: environmentDeployment.status, cancelling: environmentDeployment.cancellationRequestedAt,
  }).from(environmentDeploymentImageBuild)
    .innerJoin(environmentDeployment, eq(environmentDeployment.id, environmentDeploymentImageBuild.deploymentId))
    .where(eq(environmentDeploymentImageBuild.id, imageBuildId)).limit(1);
  return row !== undefined && row.build === "building" && row.cancelling === null && ACTIVE_ENVIRONMENT_DEPLOYMENT_STATUSES.has(row.attempt);
});

/**
 * The latest Build Receipt per image: one attempt's own, or the environment's newest as a reuse hint
 * for the next build. Receipts are optional evidence; an unreadable one just means a fresh build.
 */
export const loadBuildReceipts = Effect.fn("Deployments.loadBuildReceipts")(function* (scope: { deploymentId: string } | { environmentId: string }) {
  const { drizzle } = yield* Database;
  const encryption = yield* SecretEncryption;
  const table = environmentDeploymentImageBuild;
  const rows = yield* drizzle.selectDistinctOn([table.image], { image: table.image, receipt: table.encryptedReceipt })
    .from(table).innerJoin(environmentDeployment, eq(environmentDeployment.id, table.deploymentId))
    .where(and(eq(table.status, "built"), "deploymentId" in scope
      ? eq(table.deploymentId, scope.deploymentId)
      : eq(environmentDeployment.environmentId, scope.environmentId)))
    .orderBy(table.image, desc(table.finishedAt));
  const receipts: BuildReceipts = {};
  for (const { image, receipt } of rows) {
    if (!receipt) continue;
    const decoded = yield* Effect.try(() => Schema.decodeUnknownSync(buildReceiptSchema)(JSON.parse(encryption.decrypt(receipt)), { onExcessProperty: "error" }))
      .pipe(Effect.option);
    if (Option.isSome(decoded)) receipts[image] = decoded.value;
  }
  return receipts;
});
