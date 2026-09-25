import "@tanstack/react-start/server-only";
import { eq } from "drizzle-orm";
import { Effect } from "effect";
import { service } from "#/modules/environment-design/tables";
import type { Actor } from "#/modules/identity/actor";
import { requireInfrastructureOrganization } from "#/modules/runtime/organization-access.server";
import { OrganizationRuntime } from "#/modules/runtime/organization-runtime.server";
import { Database } from "#/server/database.server";
import { checkGithubBuildWorkflow, listOrganizationGithubRepositories } from "#/modules/github/github-build.server";
import { buildOrderCandidates, defaultBuildOrder, imageBuildWalk, type BuildCandidate, type BuildOrder, type BuildOrderRow } from "./build-order";
import { skipImageBuilder, type ImageBuildTarget } from "./image-builds.server";
import { environmentDeployment, environmentDeploymentImageBuild, organizationBuildOrder } from "./tables";

const RUNTIME_FRAME_TIMEOUT_MS = 10_000;

/** Whether GitHub is set up: some repository the Organization's Services build from has the build workflow. */
const githubBuildsSetUp = Effect.fn("Deployments.githubBuildsSetUp")(function* (organizationId: string) {
  for (const repository of yield* listOrganizationGithubRepositories(organizationId)) {
    const workflow = yield* checkGithubBuildWorkflow(repository.installationId, repository.repositoryId);
    if (workflow.readiness === "ready") return true;
  }
  return false;
}, Effect.orElseSucceed(() => false));

/** Read when an Image Build starts, so a change (or GitHub's setup, for the default) takes effect on the next build. */
export const loadBuildOrder = Effect.fn("Deployments.loadBuildOrder")(function* (organizationId: string) {
  const { drizzle } = yield* Database;
  const [row] = yield* drizzle.select({ buildOrder: organizationBuildOrder.buildOrder }).from(organizationBuildOrder)
    .where(eq(organizationBuildOrder.organizationId, organizationId)).limit(1);
  return row?.buildOrder ?? defaultBuildOrder(yield* githubBuildsSetUp(organizationId));
});

/**
 * Why a Preferred Server can't take builds any more, from the Cluster's current view; null while it
 * can. A Cluster Cloud can't see right now keeps the preference: the Engine ranks it anyway.
 */
const preferredServerUnavailable = Effect.fn("Deployments.preferredServerUnavailable")(function* (organizationId: string, machineId: string) {
  const session = yield* (yield* OrganizationRuntime).open(organizationId);
  if (session.status !== "connected") return null;
  const frame = yield* session.connected.watchFirstFrame(RUNTIME_FRAME_TIMEOUT_MS);
  const observed = frame.machines.find(({ machine }) => machine.id === machineId);
  if (!observed) return "Preferred server: no longer in the Cluster";
  return observed.machine.accepts_builds ? null : `${observed.machine.name}: no longer accepts builds`;
}, Effect.scoped, Effect.orElseSucceed(() => null));

/**
 * The Builders one Image Build tries, in turn: the Service's Preferred Builder, then the
 * Organization's Build Order without it, both read as the build starts. A Preferred Server that is
 * gone or no longer builds goes back to Auto, and the skip trail says why.
 */
export const imageBuildCandidates = Effect.fn("Deployments.imageBuildCandidates")(function* (
  build: Pick<ImageBuildTarget, "id" | "deploymentId" | "serviceId">,
) {
  const { drizzle } = yield* Database;
  const [row] = yield* drizzle.select({ organizationId: environmentDeployment.organizationId, policy: service.policy })
    .from(environmentDeployment).leftJoin(service, eq(service.id, build.serviceId))
    .where(eq(environmentDeployment.id, build.deploymentId)).limit(1);
  if (!row) return buildOrderCandidates(defaultBuildOrder(false));
  let preferred = row.policy?.preferredBuilder;
  if (preferred !== undefined && preferred !== "github") {
    const unavailable = yield* preferredServerUnavailable(row.organizationId, preferred);
    if (unavailable !== null) {
      yield* skipImageBuilder(build.id, unavailable);
      preferred = undefined;
    }
  }
  if (preferred !== undefined) {
    yield* drizzle.update(environmentDeploymentImageBuild).set({ preferred: true })
      .where(eq(environmentDeploymentImageBuild.id, build.id));
  }
  const candidates: BuildCandidate[] = imageBuildWalk(yield* loadBuildOrder(row.organizationId), preferred);
  return candidates;
});

export const setBuildOrder = Effect.fn("Deployments.setBuildOrder")(function* (
  actor: Actor,
  input: { organizationSlug: string; buildOrder: BuildOrder },
) {
  const organization = yield* requireInfrastructureOrganization(actor, input.organizationSlug);
  const { drizzle } = yield* Database;
  yield* drizzle.insert(organizationBuildOrder).values({ organizationId: organization.id, buildOrder: input.buildOrder })
    .onConflictDoUpdate({ target: organizationBuildOrder.organizationId, set: { buildOrder: input.buildOrder, updatedAt: new Date() } });
  const row: BuildOrderRow = { id: organization.id, buildOrder: input.buildOrder };
  return row;
});
