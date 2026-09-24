import type { ChangeName } from "./read.contract";

/**
 * The one map from a change-log source table to the names it feeds. Every organization-owned table
 * is here, with the key columns its trigger logs (joined with ':'); tables no client reads feed nothing.
 * One key per table means collections sharing a table share its key, and collection reads match it.
 */
export const changeSources = {
  organization: { key: ["id"], feeds: ["organization"] },
  project: { key: ["id"], feeds: ["project"] },
  environment: { key: ["id"], feeds: ["environment", "environment_summary"] },
  user_project_preference: { key: ["project_id"], feeds: ["project_preference"] },
  service: { key: ["id"], feeds: ["service"] },
  resource_lineage: { key: ["id"], feeds: ["resource_lineage"] },
  environment_resource: { key: ["id"], feeds: ["environment_resource"] },
  environment_canvas_node_position: { key: ["resource_type", "resource_id"], feeds: ["environment_canvas_node_position"] },
  environment_deployment: { key: ["id"], feeds: ["environment_deployment"] },
  environment_deployment_event: { key: ["deployment_id"], feeds: ["environment_deployment"] },
  environment_saved_state_snapshot: { key: ["id"], feeds: ["environment_saved_state_snapshot"] },
  environment_node_config_snapshot: { key: ["id"], feeds: ["environment_node_config_snapshot"] },
  environment_node_introduction: { key: ["node_type", "node_id"], feeds: ["environment_node_introduction"] },
  volume_remove_attempt: { key: ["id"], feeds: ["volume_remove_attempt"] },
  organization_pairing: { key: ["organization_id"], feeds: ["organization_enrollment"] },
  core_operation_event: { key: ["id"], feeds: [] },
  core_operation_watch: { key: ["id"], feeds: [] },
  enrollment_allocation: { key: ["cluster_key"], feeds: [] },
  environment_deployment_build_output: { key: ["id"], feeds: [] },
  environment_deployment_build_step: { key: ["id"], feeds: [] },
  environment_deployment_secret: { key: ["environment_deployment_id"], feeds: [] },
  environment_node_config_snapshot_secret: { key: ["snapshot_id"], feeds: [] },
  environment_node_introduction_secret: { key: ["environment_id", "node_type", "node_id"], feeds: [] },
  github_environment_trigger: { key: ["id"], feeds: [] },
  invitation: { key: ["id"], feeds: [] },
  machine_enrollment_token: { key: ["id"], feeds: [] },
  machine_remove_attempt: { key: ["id"], feeds: [] },
  member: { key: ["id"], feeds: [] },
  organization_billing_state: { key: ["organization_id"], feeds: [] },
  organization_machine: { key: ["machine_id"], feeds: [] },
  service_lineage: { key: ["id"], feeds: [] },
  service_registry_credential: { key: ["service_id"], feeds: [] },
  teardown_attempt: { key: ["id"], feeds: [] },
  variable: { key: ["id"], feeds: [] },
  variable_secret: { key: ["variable_id"], feeds: [] },
} satisfies Record<string, { key: readonly string[]; feeds: readonly ChangeName[] }>;

export type ChangeSource = keyof typeof changeSources;

// SAFETY: changeSources is an object literal, so its own keys are exactly ChangeSource.
const sources = Object.entries(changeSources) as [ChangeSource, { key: readonly string[]; feeds: readonly ChangeName[] }][];

export function sourceTablesOf(name: ChangeName) {
  return sources.filter(([, source]) => source.feeds.some((feed) => feed === name)).map(([table]) => table);
}

export function collectionsOf(sourceTables: Iterable<ChangeSource>) {
  const tables = new Set(sourceTables);
  return [...new Set(sources.filter(([table]) => tables.has(table)).flatMap(([, source]) => source.feeds))];
}
