import "@tanstack/react-start/server-only";
import { eq } from "drizzle-orm";
import { Effect } from "effect";
import type { Actor } from "#/modules/identity/actor";
import { requireInfrastructureOrganization } from "#/modules/runtime/organization-access.server";
import { Database } from "#/server/database.server";
import { buildOrderCandidates, DEFAULT_BUILD_ORDER, type BuildCandidate, type BuildOrder, type BuildOrderRow } from "./build-order";
import { environmentDeployment, organizationBuildOrder } from "./tables";

/** Read when an Image Build starts, so a change takes effect on the next build. */
export const loadBuildOrder = Effect.fn("Deployments.loadBuildOrder")(function* (organizationId: string) {
  const { drizzle } = yield* Database;
  const [row] = yield* drizzle.select({ buildOrder: organizationBuildOrder.buildOrder }).from(organizationBuildOrder)
    .where(eq(organizationBuildOrder.organizationId, organizationId)).limit(1);
  return row?.buildOrder ?? DEFAULT_BUILD_ORDER;
});

/**
 * The Builders one Image Build tries, in turn: the Organization's Build Order, read as the build
 * starts. A Service's preferred builder goes in front of these.
 */
export const imageBuildCandidates = Effect.fn("Deployments.imageBuildCandidates")(function* (build: { deploymentId: string }) {
  const { drizzle } = yield* Database;
  const [deployment] = yield* drizzle.select({ organizationId: environmentDeployment.organizationId }).from(environmentDeployment)
    .where(eq(environmentDeployment.id, build.deploymentId)).limit(1);
  const candidates: BuildCandidate[] = buildOrderCandidates(deployment ? yield* loadBuildOrder(deployment.organizationId) : DEFAULT_BUILD_ORDER);
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
