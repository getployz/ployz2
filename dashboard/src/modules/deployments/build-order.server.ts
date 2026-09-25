import "@tanstack/react-start/server-only";
import { eq } from "drizzle-orm";
import { Effect } from "effect";
import type { Actor } from "#/modules/identity/actor";
import { requireInfrastructureOrganization } from "#/modules/runtime/organization-access.server";
import { Database } from "#/server/database.server";
import { DEFAULT_BUILD_ORDER, type BuildOrder, type BuildOrderRow } from "./build-order";
import { organizationBuildOrder } from "./tables";

/** Read when an Image Build starts, so a change takes effect on the next build. */
export const loadBuildOrder = Effect.fn("Deployments.loadBuildOrder")(function* (organizationId: string) {
  const { drizzle } = yield* Database;
  const [row] = yield* drizzle.select({ buildOrder: organizationBuildOrder.buildOrder }).from(organizationBuildOrder)
    .where(eq(organizationBuildOrder.organizationId, organizationId)).limit(1);
  return row?.buildOrder ?? DEFAULT_BUILD_ORDER;
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
