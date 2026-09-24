import "@tanstack/react-start/server-only";
import { and, eq, gte, inArray, lt, lte, sql } from "drizzle-orm";
import { Data, Effect } from "effect";
import type { ChangeSource } from "#/modules/organization/change-log.sources";
import { organizationChange as change } from "#/modules/organization/tables";
import { Database } from "#/server/database.server";

export class OrganizationChangeLogFailure extends Data.TaggedError("OrganizationChangeLogFailure")<{
  readonly cause: unknown;
}> {}

/**
 * Everything an Organization logged between `since` and `cursor`, merged across log rows.
 * `full` means read everything: there was no `since`, a logged statement touched too many rows
 * to name them, or retention deleted changes the window may need (`expired`, never set on a delta). Both kinds name
 * the source tables that logged changes, so the change stream can still name their collections.
 */
export type ChangeWindow =
  | { kind: "full"; cursor: string; expired: boolean; sourceTables: ChangeSource[] }
  | { kind: "delta"; cursor: string; expired: false; sourceTables: ChangeSource[]; changed: string[]; deleted: string[] };

/**
 * The cursor is the xid horizon, not the seq: seq order is insert order, so a transaction
 * with an older xid can log a later seq, and a seq cursor would skip the earlier one.
 * Every transaction below the horizon has finished, so a window never misses a row, and
 * a slow transaction only delays the windows after it.
 *
 * Retention keeps every row at or above some xid, so the oldest retained xid is a fence:
 * a `since` below it, or any `since` against an empty log, may have lost changes.
 */
export const readChangeWindow = Effect.fn("OrganizationChangeLog.readWindow")(function* (input: {
  organizationId: string;
  since: string | undefined;
  sourceTables?: readonly ChangeSource[];
}) {
  const database = yield* Database;
  // Without `since` the reader starts at the horizon and sees no rows.
  const since = input.since ?? null;
  // Source tables are ChangeSource: only the triggers attached to changeSources' tables write the log.
  const [window] = yield* database.drizzle.execute<{
    cursor: string;
    expired: boolean;
    sourceTables: ChangeSource[];
    fullRead: boolean;
    changed: string[];
    deleted: string[];
  }>(sql`
    with horizon as (select pg_snapshot_xmin(pg_current_snapshot()) as xid),
      logged as (select * from ${change} where ${and(
        eq(change.organizationId, input.organizationId),
        gte(change.xid, sql`${since}::xid8`),
        sql`${change.xid} < (select xid from horizon)`,
        input.sourceTables ? inArray(change.sourceTable, [...input.sourceTables]) : undefined,
      )})
    select (select xid::text from horizon) as "cursor",
      coalesce(${since}::xid8 < (select min(${change.xid}) from ${change}), ${since}::xid8 is not null) as "expired",
      array(select distinct source_table from logged) as "sourceTables",
      coalesce((select bool_or(all_rows) from logged), false) as "fullRead",
      array(select distinct unnest(changed_ids) from logged) as "changed",
      array(select distinct unnest(deleted_ids) from logged) as "deleted"
  `, "objects").pipe(Effect.mapError((cause) => new OrganizationChangeLogFailure({ cause })));
  if (!window) return yield* new OrganizationChangeLogFailure({ cause: "The change window query returned no row." });
  const { cursor, expired, sourceTables, changed, deleted } = window;
  if (input.since === undefined || window.fullRead || expired) return { kind: "full", cursor, expired, sourceTables } satisfies ChangeWindow;
  return { kind: "delta", cursor, expired: false, sourceTables, changed, deleted } satisfies ChangeWindow;
});

/**
 * Deletes changes older than 24 hours by xid, never at or past the horizon, so every row
 * left is newer than every row deleted and readers need no watermark. The newest logged
 * transaction always stays, so a quiet log keeps its fence instead of emptying.
 */
export const pruneChangeLog = Effect.fn("OrganizationChangeLog.prune")(function* () {
  const database = yield* Database;
  const retentionFence = database.drizzle.select({ xid: sql`max(${change.xid})` }).from(change)
    .where(lt(change.createdAt, sql`now() - interval '24 hours'`));
  const newest = database.drizzle.select({ xid: sql`max(${change.xid})` }).from(change);
  yield* database.drizzle.delete(change).where(and(
    lte(change.xid, sql`(${retentionFence})`),
    lt(change.xid, sql`(${newest})`),
    lt(change.xid, sql`pg_snapshot_xmin(pg_current_snapshot())`),
  )).pipe(Effect.mapError((cause) => new OrganizationChangeLogFailure({ cause })));
});
