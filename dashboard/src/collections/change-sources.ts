import * as EffectRecord from "effect/Record";
import type { ChangeName } from "./read.contract";

/**
 * Every organization-owned table, with the key columns its change trigger logs (joined with ':').
 * The spec logs every organization-owned table (#1042 user story 21), including tables that feed no
 * collection yet. The migration attaches each trigger with these columns; the ownership test checks they match.
 */
export const changeSources = {
  organization: ["id"],
  project: ["id"],
  environment: ["id"],
  user_project_preference: ["project_id"],
  service: ["id"],
  resource_lineage: ["id"],
  environment_resource: ["id"],
  environment_canvas_node_position: ["resource_type", "resource_id"],
  environment_deployment: ["id"],
  environment_deployment_event: ["deployment_id"],
  environment_saved_state_snapshot: ["id"],
  environment_node_config_snapshot: ["id"],
  environment_node_introduction: ["node_type", "node_id"],
  volume_remove_attempt: ["id"],
  organization_pairing: ["organization_id"],
  core_operation_event: ["id"],
  core_operation_watch: ["id"],
  enrollment_allocation: ["cluster_key"],
  environment_deployment_build_output: ["id"],
  environment_deployment_build_step: ["id"],
  environment_deployment_secret: ["environment_deployment_id"],
  environment_node_config_snapshot_secret: ["snapshot_id"],
  environment_node_introduction_secret: ["environment_id", "node_type", "node_id"],
  github_environment_trigger: ["id"],
  invitation: ["id"],
  machine_enrollment_token: ["id"],
  machine_remove_attempt: ["id"],
  member: ["id"],
  organization_billing_state: ["organization_id"],
  organization_machine: ["machine_id"],
  service_lineage: ["id"],
  service_registry_credential: ["service_id"],
  teardown_attempt: ["id"],
  variable: ["id"],
  variable_secret: ["variable_id"],
} satisfies Record<string, readonly string[]>;

export type ChangeSource = keyof typeof changeSources;

/**
 * The source tables each change stream name reads: an Org Store collection or `organization`. The first is
 * its key table: the collection's rows are keyed by the key that table logs, and every other source
 * logs that same key through a foreign key to it.
 */
export const changeNameSources = {
  organization: ["organization"],
  project: ["project"],
  environment: ["environment"],
  environment_summary: ["environment"],
  project_preference: ["user_project_preference"],
  service: ["service"],
  resource_lineage: ["resource_lineage"],
  environment_resource: ["environment_resource"],
  environment_canvas_node_position: ["environment_canvas_node_position"],
  environment_deployment: ["environment_deployment", "environment_deployment_event"],
  environment_saved_state_snapshot: ["environment_saved_state_snapshot"],
  environment_node_config_snapshot: ["environment_node_config_snapshot"],
  environment_node_introduction: ["environment_node_introduction"],
  volume_remove_attempt: ["volume_remove_attempt"],
  organization_enrollment: ["organization_pairing"],
} satisfies Record<ChangeName, readonly [ChangeSource, ...ChangeSource[]]>;

export function collectionsOf(sourceTables: Iterable<ChangeSource>) {
  const tables = new Set(sourceTables);
  return EffectRecord.keys(changeNameSources).filter((name) => changeNameSources[name].some((table) => tables.has(table)));
}
