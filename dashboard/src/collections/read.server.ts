import "@tanstack/react-start/server-only";
import { and, eq, getTableColumns, inArray, sql } from "drizzle-orm";
import { Data, Effect } from "effect";
import { sourceTablesOf } from "./change-sources";
import { readChangeWindow } from "./changes.server";
import type { CollectionRead, CollectionReadInput } from "./read.contract";
import * as tables from "#/db/schema";
import type { Actor } from "#/modules/identity/actor";
import { pairingEnrollmentStatus, type OrganizationEnrollmentRow } from "#/modules/machines/enrollment";
import { getOrganizationForUserBySlug } from "#/modules/environment-design/workspace-repository.server";
import { Database } from "#/server/database.server";

export class CollectionReadFailure extends Data.TaggedError("CollectionReadFailure")<{
  readonly cause: unknown;
}> {
  readonly publicErrorCategory = "internal" as const;
}
export class CollectionReadDenied extends Data.TaggedError("CollectionReadDenied")<{
  readonly message: string;
}> {
  readonly publicErrorCategory = "not-found" as const;
}
export class CollectionReadInvalid extends Data.TaggedError("CollectionReadInvalid")<{
  readonly message: string;
}> {
  readonly publicErrorCategory = "validation" as const;
}

export const readCollection = Effect.fn("Collections.read")(function* (
  actor: Actor,
  data: CollectionReadInput,
) {
  if (actor.userId !== data.userId) {
    return yield* new CollectionReadDenied({ message: "Collection not found." });
  }
  let scopeId = actor.userId;
  if (data.table !== "github_repository_cache") {
    if (!data.organizationSlug) {
      return yield* new CollectionReadInvalid({ message: "organizationSlug is required." });
    }
    const organization = yield* getOrganizationForUserBySlug(actor.userId, data.organizationSlug)
      .pipe(Effect.mapError((cause) => new CollectionReadFailure({ cause })));
    if (!organization) {
      return yield* new CollectionReadDenied({ message: "Organization not found." });
    }
    scopeId = organization.id;
  }
  const database = yield* Database;
  // `keys` narrows a sourced collection to the rows its change window names.
  const readRows = (keys?: string[]) => Effect.gen(function* () {
    switch (data.table) {
      case "github_repository_cache":
        return yield* database.drizzle.select().from(tables.githubRepositoryCache)
          .where(eq(tables.githubRepositoryCache.userId, scopeId));
      case "environment_saved_state_snapshot":
        return yield* database.drizzle.select({
          id: tables.environmentSavedStateSnapshot.id,
          organizationId: tables.environmentSavedStateSnapshot.organizationId,
          environmentId: tables.environmentSavedStateSnapshot.environmentId,
        }).from(tables.environmentSavedStateSnapshot)
          .where(eq(tables.environmentSavedStateSnapshot.organizationId, scopeId));
      case "environment_summary":
        return yield* database.drizzle.select({
          id: tables.environment.id, projectId: tables.environment.projectId, organizationId: tables.environment.organizationId,
          name: tables.environment.name, namespace: tables.environment.namespace, createdAt: tables.environment.createdAt,
        }).from(tables.environment).where(eq(tables.environment.organizationId, scopeId));
      case "project_preference":
        return yield* database.drizzle.select({ id: tables.userProjectPreference.projectId, environmentId: tables.userProjectPreference.environmentId })
          .from(tables.userProjectPreference).where(and(eq(tables.userProjectPreference.organizationId, scopeId), eq(tables.userProjectPreference.userId, actor.userId)));
      case "project":
        return yield* database.drizzle.select().from(tables.project)
          .where(eq(tables.project.organizationId, scopeId));
      case "environment":
        return yield* database.drizzle.select().from(tables.environment)
          .where(eq(tables.environment.organizationId, scopeId));
      case "service":
        return yield* database.drizzle.select().from(tables.service)
          .where(and(eq(tables.service.organizationId, scopeId), keys && inArray(tables.service.id, keys)));
      case "resource_lineage":
        return yield* database.drizzle.select().from(tables.resourceLineage)
          .where(eq(tables.resourceLineage.organizationId, scopeId));
      case "environment_resource":
        return yield* database.drizzle.select().from(tables.environmentResource)
          .where(eq(tables.environmentResource.organizationId, scopeId));
      case "environment_canvas_node_position":
        return yield* database.drizzle.select().from(tables.environmentCanvasNodePosition)
          .where(eq(tables.environmentCanvasNodePosition.organizationId, scopeId));
      case "environment_deployment":
        return yield* database.drizzle.select({
          ...getTableColumns(tables.environmentDeployment),
          runtimeProgress: sql<typeof tables.environmentDeployment.$inferSelect.runtimeProgress>`coalesce(
            ${tables.environmentDeployment.runtimeProgress},
            (select progress from ${tables.environmentDeploymentEvent}
             where deployment_id = ${tables.environmentDeployment}.${sql.identifier("id")} order by id desc limit 1)
          )`,
        }).from(tables.environmentDeployment)
          .where(eq(tables.environmentDeployment.organizationId, scopeId));
      case "environment_node_config_snapshot":
        return yield* database.drizzle.select().from(tables.environmentNodeConfigSnapshot)
          .where(eq(tables.environmentNodeConfigSnapshot.organizationId, scopeId));
      case "environment_node_introduction":
        return yield* database.drizzle.select().from(tables.environmentNodeIntroduction)
          .where(eq(tables.environmentNodeIntroduction.organizationId, scopeId));
      case "volume_remove_attempt":
        return yield* database.drizzle.select().from(tables.volumeRemoveAttempt)
          .where(eq(tables.volumeRemoveAttempt.organizationId, scopeId));
      case "organization_enrollment": {
        // The pairing row holds the encrypted pairing secret; expose only the derived status.
        const pairings = yield* database.drizzle.select({
          id: tables.organizationPairing.organizationId,
          founderMachineId: tables.organizationPairing.founderMachineId,
        }).from(tables.organizationPairing).where(eq(tables.organizationPairing.organizationId, scopeId));
        return pairings.map((row): OrganizationEnrollmentRow => ({ id: row.id, status: pairingEnrollmentStatus(row.founderMachineId) }));
      }
    }
  });
  type Row = Effect.Success<ReturnType<typeof readRows>>[number];
  const read = Effect.gen(function* (): Effect.fn.Return<CollectionRead<Row>, unknown, Database> {
    const sourceTables = sourceTablesOf(data.table);
    if (sourceTables.length === 0) return { full: true, rows: yield* readRows(), cursor: null };
    // The window is read before the rows, so the rows are at least as new as its cursor.
    const window = yield* readChangeWindow({ organizationId: scopeId, since: data.since, sourceTables });
    if (data.since === undefined || window.all) return { full: true, rows: yield* readRows(), cursor: window.cursor };
    const rows = window.changed.length === 0 ? [] : yield* readRows(window.changed);
    return { full: false, rows, deleted: window.deleted, cursor: window.cursor };
  });
  return yield* read.pipe(Effect.mapError((cause) => new CollectionReadFailure({ cause })));
});
