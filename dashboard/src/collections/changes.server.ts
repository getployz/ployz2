import "@tanstack/react-start/server-only";
import { and, eq, gte, inArray, sql } from "drizzle-orm";
import { Effect } from "effect";
import { organizationChange } from "#/modules/organization/tables";
import { Database } from "#/server/database.server";

/** Everything an Organization logged between `since` and `cursor`, merged across log rows. */
export type ChangeWindow = {
  cursor: string;
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
    source_table: string | null;
    all_rows: boolean | null;
    changed_ids: string[] | null;
    deleted_ids: string[] | null;
  }>(sql`
    with horizon as (select pg_snapshot_xmin(pg_current_snapshot()) as xid)
    select horizon.xid::text as cursor, ${change.sourceTable}, ${change.allRows}, ${change.changedIds}, ${change.deletedIds}
    from horizon left join ${change} on ${and(
      eq(change.organizationId, input.organizationId),
      gte(change.xid, sql`${since}::xid8`),
      sql`${change.xid} < horizon.xid`,
      input.sourceTables ? inArray(change.sourceTable, [...input.sourceTables]) : undefined,
    )}
  `, "objects");
  const window: ChangeWindow = { cursor: rows[0]?.cursor ?? "0", sourceTables: [], all: false, changed: [], deleted: [] };
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
