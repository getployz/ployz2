import { Schema } from "effect";

/** An xid8 horizon from the Organization change log: every change before it has been read. */
export const changeCursorSchema = Schema.String.check(Schema.isPattern(/^\d{1,20}$/u));

export const collectionReadInput = Schema.Struct({
  table: Schema.Literals([
    "project", "environment_summary", "project_preference", "environment", "service", "resource_lineage",
    "environment_resource", "environment_canvas_node_position",
    "environment_deployment", "environment_saved_state_snapshot",
    "environment_node_config_snapshot", "environment_node_introduction",
    "volume_remove_attempt", "organization_enrollment", "github_repository_cache",
  ]),
  userId: Schema.String,
  organizationSlug: Schema.optional(Schema.String),
  since: Schema.optional(changeCursorSchema),
});

export type CollectionReadInput = typeof collectionReadInput.Type;
export type CollectionName = CollectionReadInput["table"];

/**
 * A collection read. `full` replaces every row; otherwise drop `deleted`, then upsert `rows`.
 * `cursor` is the next `since`; null means the collection has no change source and always reads in full.
 */
export type CollectionRead<Row> =
  | { full: true; rows: Row[]; cursor: string | null }
  | { full: false; rows: Row[]; deleted: string[]; cursor: string };
