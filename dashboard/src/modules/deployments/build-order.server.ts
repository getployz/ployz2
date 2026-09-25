import "@tanstack/react-start/server-only";
import { eq } from "drizzle-orm";
import { Effect, Option, Schema } from "effect";
import { servicePolicySchema } from "#/modules/environment-design/service-policy";
import { service } from "#/modules/environment-design/tables";
import { checkGithubBuildWorkflow, listOrganizationGithubRepositories } from "#/modules/github/github-build.server";
import type { Actor } from "#/modules/identity/actor";
import { requireInfrastructureOrganization } from "#/modules/runtime/organization-access.server";
import { Database } from "#/server/database.server";
import { defaultBuildOrder, imageBuildWalk, type BuildOrder, type BuildOrderRow } from "./build-order";
import type { ImageBuildTarget } from "./image-builds.server";
import { environmentDeployment, organizationBuildOrder } from "./tables";

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
 * The Builders one Image Build tries, in turn: the Service's Preferred Builder, then the
 * Organization's Build Order without it, both read as the build starts. A Preferred Server the
 * Cluster can't use any more is the Engine's to notice: it chooses as it would without it, and says so.
 */
export const imageBuildCandidates = Effect.fn("Deployments.imageBuildCandidates")(function* (
  build: Pick<ImageBuildTarget, "deploymentId" | "serviceId">,
) {
  const { drizzle } = yield* Database;
  const [row] = yield* drizzle.select({ organizationId: environmentDeployment.organizationId, policy: service.policy })
    .from(environmentDeployment).leftJoin(service, eq(service.id, build.serviceId))
    .where(eq(environmentDeployment.id, build.deploymentId)).limit(1);
  if (!row) return imageBuildWalk(defaultBuildOrder(false), undefined);
  const policy = Schema.decodeUnknownOption(servicePolicySchema)(row.policy);
  const preferred = Option.isSome(policy) ? policy.value.preferredBuilder : undefined;
  return imageBuildWalk(yield* loadBuildOrder(row.organizationId), preferred);
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
