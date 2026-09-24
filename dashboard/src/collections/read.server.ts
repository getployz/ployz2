import "@tanstack/react-start/server-only";
import { and, eq, getTableColumns, inArray, sql } from "drizzle-orm";
import type { EffectDrizzleQueryError } from "drizzle-orm/effect-core";
import type { AnyPgColumn, PgTable } from "drizzle-orm/pg-core";
import { Data, Effect } from "effect";
import { changeSources, sourceTablesOf, type ChangeSource } from "./change-sources";
import type { CollectionRead, CollectionReadInput } from "./read.contract";
import * as tables from "#/db/schema";
import type { Actor } from "#/modules/identity/actor";
import { pairingEnrollmentStatus, type OrganizationEnrollmentRow } from "#/modules/machines/enrollment";
import { readChangeWindow, type OrganizationChangeLogFailure } from "#/modules/organization/change-log.server";
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

export const readCollection = Effect.fn("Collections.read")(function* (
  actor: Actor,
  data: CollectionReadInput,
) {
  if (actor.userId !== data.userId) {
    return yield* new CollectionReadDenied({ message: "Collection not found." });
  }
  const organization = yield* getOrganizationForUserBySlug(actor.userId, data.organizationSlug)
    .pipe(Effect.mapError((cause) => new CollectionReadFailure({ cause })));
  if (!organization) {
    return yield* new CollectionReadDenied({ message: "Organization not found." });
  }
  const organizationId = organization.id;
  const database = yield* Database;
  // `keys` narrows a read to the rows its change window names, by the key `source` logs.
  const readRows = (keys?: string[]) => Effect.gen(function* () {
    const scoped = (table: PgTable & { organizationId: AnyPgColumn }, source: ChangeSource) => and(
      eq(table.organizationId, organizationId),
      keys && inArray(sql.join(changeSources[source].key.map((column) => sql`${table}.${sql.identifier(column)}`), sql` || ':' || `), keys),
    );
    switch (data.table) {
      case "environment_saved_state_snapshot":
        return yield* database.drizzle.select({
          id: tables.environmentSavedStateSnapshot.id,
          organizationId: tables.environmentSavedStateSnapshot.organizationId,
          environmentId: tables.environmentSavedStateSnapshot.environmentId,
        }).from(tables.environmentSavedStateSnapshot)
          .where(scoped(tables.environmentSavedStateSnapshot, "environment_saved_state_snapshot"));
      case "environment_summary":
        return yield* database.drizzle.select({
          id: tables.environment.id, projectId: tables.environment.projectId, organizationId: tables.environment.organizationId,
          name: tables.environment.name, namespace: tables.environment.namespace, createdAt: tables.environment.createdAt,
        }).from(tables.environment).where(scoped(tables.environment, "environment"));
      case "project_preference":
        return yield* database.drizzle.select({ id: tables.userProjectPreference.projectId, environmentId: tables.userProjectPreference.environmentId })
          .from(tables.userProjectPreference)
          .where(and(scoped(tables.userProjectPreference, "user_project_preference"), eq(tables.userProjectPreference.userId, actor.userId)));
      case "project":
        return yield* database.drizzle.select().from(tables.project).where(scoped(tables.project, "project"));
      case "environment":
        return yield* database.drizzle.select().from(tables.environment).where(scoped(tables.environment, "environment"));
      case "service":
        return yield* database.drizzle.select().from(tables.service).where(scoped(tables.service, "service"));
      case "resource_lineage":
        return yield* database.drizzle.select().from(tables.resourceLineage).where(scoped(tables.resourceLineage, "resource_lineage"));
      case "environment_resource":
        return yield* database.drizzle.select().from(tables.environmentResource)
          .where(scoped(tables.environmentResource, "environment_resource"));
      case "environment_canvas_node_position":
        return yield* database.drizzle.select().from(tables.environmentCanvasNodePosition)
          .where(scoped(tables.environmentCanvasNodePosition, "environment_canvas_node_position"));
      case "environment_deployment":
        return yield* database.drizzle.select({
          ...getTableColumns(tables.environmentDeployment),
          runtimeProgress: sql<typeof tables.environmentDeployment.$inferSelect.runtimeProgress>`coalesce(
            ${tables.environmentDeployment.runtimeProgress},
            (select progress from ${tables.environmentDeploymentEvent}
             where deployment_id = ${tables.environmentDeployment}.${sql.identifier("id")} order by id desc limit 1)
          )`,
        }).from(tables.environmentDeployment).where(scoped(tables.environmentDeployment, "environment_deployment"));
      case "environment_node_config_snapshot":
        return yield* database.drizzle.select().from(tables.environmentNodeConfigSnapshot)
          .where(scoped(tables.environmentNodeConfigSnapshot, "environment_node_config_snapshot"));
      case "environment_node_introduction":
        return yield* database.drizzle.select().from(tables.environmentNodeIntroduction)
          .where(scoped(tables.environmentNodeIntroduction, "environment_node_introduction"));
      case "volume_remove_attempt":
        return yield* database.drizzle.select().from(tables.volumeRemoveAttempt)
          .where(scoped(tables.volumeRemoveAttempt, "volume_remove_attempt"));
      case "organization_enrollment": {
        // The pairing row holds the encrypted pairing secret; expose only the derived status.
        const pairings = yield* database.drizzle.select({
          id: tables.organizationPairing.organizationId,
          founderMachineId: tables.organizationPairing.founderMachineId,
        }).from(tables.organizationPairing).where(scoped(tables.organizationPairing, "organization_pairing"));
        return pairings.map((row): OrganizationEnrollmentRow => ({ id: row.id, status: pairingEnrollmentStatus(row.founderMachineId) }));
      }
    }
  });
  type Row = Effect.Success<ReturnType<typeof readRows>>[number];
  const read = Effect.gen(function* (): Effect.fn.Return<CollectionRead<Row>, EffectDrizzleQueryError | OrganizationChangeLogFailure, Database> {
    // The window is read before the rows, so the rows are at least as new as its cursor.
    const window = yield* readChangeWindow({ organizationId, since: data.since, sourceTables: sourceTablesOf(data.table) });
    if (data.since === undefined || window.fullRead || window.expired) return { full: true, rows: yield* readRows(), cursor: window.cursor };
    // Deleted keys are re-read too: a key a filtered read shares with another user's row
    // (project preferences) can be deleted there and still exist here. The client drops, then upserts.
    const keys = [...new Set([...window.changed, ...window.deleted])];
    const rows = keys.length === 0 ? [] : yield* readRows(keys);
    return { full: false, rows, deleted: window.deleted, cursor: window.cursor };
  });
  return yield* read.pipe(Effect.mapError((cause) => new CollectionReadFailure({ cause })));
});
