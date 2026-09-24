import "@tanstack/react-start/server-only";
import { and, eq, gte, inArray, lt, lte, sql } from "drizzle-orm";
import { Effect } from "effect";
import { organizationChange } from "#/modules/organization/tables";
import { Database } from "#/server/database.server";

/** Everything an Organization logged between `since` and `cursor`, merged across log rows. */
export type ChangeWindow = {
  cursor: string;
  /** Retention deleted changes this window may need: read everything instead. */
  expired: boolean;
  sourceTables: string[];
  all: boolean;
  changed: string[];
  deleted: string[];
};

/**
 * The cursor is the xid horizon, not the seq: seq order is insert order, so a transaction
 * with an older xid can log a later seq, and a seq cursor would skip the earlier one.
 * Every transaction below the horizon has finished, so a window never misses a row, and
 * a slow transaction only delays the windows after it.
 *
 * Retention keeps every row at or above some xid, so the oldest retained xid is a fence:
 * a `since` below it, or any `since` against an empty log, may have lost changes.
 */
export const readChangeWindow = Effect.fn("OrganizationChanges.readWindow")(function* (input: {
  organizationId: string;
  since: string | undefined;
  sourceTables?: readonly string[];
}) {
  const database = yield* Database;
  // Without `since` the reader starts at the horizon and sees no rows.
  const since = input.since ?? null;
  const change = organizationChange;
  const rows = yield* database.drizzle.execute<{
    cursor: string;
    expired: boolean;
    source_table: string | null;
    all_rows: boolean | null;
    changed_ids: string[] | null;
    deleted_ids: string[] | null;
  }>(sql`
    with horizon as (select pg_snapshot_xmin(pg_current_snapshot()) as xid),
      fence as (select min(${change.xid}) as xid from ${change})
    select horizon.xid::text as cursor,
      coalesce(${since}::xid8 < fence.xid, ${since}::xid8 is not null) as expired,
      ${change.sourceTable}, ${change.allRows}, ${change.changedIds}, ${change.deletedIds}
    from horizon cross join fence left join ${change} on ${and(
      eq(change.organizationId, input.organizationId),
      gte(change.xid, sql`${since}::xid8`),
      sql`${change.xid} < horizon.xid`,
      input.sourceTables ? inArray(change.sourceTable, [...input.sourceTables]) : undefined,
    )}
  `, "objects");
  const window: ChangeWindow = {
    cursor: rows[0]?.cursor ?? "0", expired: rows[0]?.expired ?? false, sourceTables: [], all: false, changed: [], deleted: [],
  };
  for (const row of rows) {
    if (row.source_table === null) continue;
    window.sourceTables.push(row.source_table);
    window.all ||= row.all_rows === true;
    window.changed.push(...row.changed_ids ?? []);
    window.deleted.push(...row.deleted_ids ?? []);
  }
  return {
    ...window,
    sourceTables: [...new Set(window.sourceTables)],
    changed: [...new Set(window.changed)],
    deleted: [...new Set(window.deleted)],
  };
});

/**
 * Deletes changes older than 24 hours by xid, never at or past the horizon, so every row
 * left is newer than every row deleted and readers need no watermark.
 */
export const pruneChangeLog = Effect.fn("OrganizationChanges.prune")(function* () {
  const database = yield* Database;
  const change = organizationChange;
  const old = database.drizzle.select({ xid: sql`max(${change.xid})` }).from(change)
    .where(lt(change.createdAt, sql`now() - interval '24 hours'`));
  yield* database.drizzle.delete(change).where(and(
    lte(change.xid, sql`(${old})`),
    lt(change.xid, sql`pg_snapshot_xmin(pg_current_snapshot())`),
  ));
});
