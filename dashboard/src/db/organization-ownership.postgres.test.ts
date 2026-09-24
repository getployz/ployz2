import { afterAll, beforeAll, expect, it } from "vitest";
import {
  type GithubPostgresTestHarness,
  startGithubPostgresTestHarness,
} from "#/modules/github/github-ingestion.postgres-test-harness";

/**
 * Every row of an organization-owned table belongs to exactly one Organization,
 * directly or through its parent row, and stores it in this column.
 */
const organizationOwned = {
  organization: "id",
  core_operation_event: "organization_id",
  core_operation_watch: "organization_id",
  enrollment_allocation: "organization_id",
  environment: "organization_id",
  environment_canvas_node_position: "organization_id",
  environment_deployment: "organization_id",
  environment_deployment_build_output: "organization_id",
  environment_deployment_build_step: "organization_id",
  environment_deployment_event: "organization_id",
  environment_deployment_secret: "organization_id",
  environment_node_config_snapshot: "organization_id",
  environment_node_config_snapshot_secret: "organization_id",
  environment_node_introduction: "organization_id",
  environment_node_introduction_secret: "organization_id",
  environment_resource: "organization_id",
  environment_saved_state_snapshot: "organization_id",
  github_environment_trigger: "organization_id",
  invitation: "organization_id",
  machine_enrollment_token: "organization_id",
  machine_remove_attempt: "organization_id",
  member: "organization_id",
  organization_billing_state: "organization_id",
  organization_machine: "organization_id",
  organization_pairing: "organization_id",
  project: "organization_id",
  resource_lineage: "organization_id",
  service: "organization_id",
  service_lineage: "organization_id",
  service_registry_credential: "organization_id",
  teardown_attempt: "organization_id",
  user_project_preference: "organization_id",
  variable: "organization_id",
  variable_secret: "organization_id",
  volume_remove_attempt: "organization_id",
} satisfies Record<string, string>;

const notOrganizationOwned = {
  user: "Belongs to a user.",
  account: "Belongs to a user.",
  verification: "Belongs to a user.",
  session: "Belongs to a user; its active Organization doesn't make it organization-owned.",
  github_installation: "Belongs to a user.",
  github_repository_cache: "Belongs to a user.",
  github_branch_projection: "Belongs to a GitHub installation, which several Organizations can share.",
  github_check_suite_projection: "Belongs to a GitHub installation, which several Organizations can share.",
  github_webhook_delivery: "Belongs to a GitHub installation, which several Organizations can share.",
  waitlist: "Belongs to no one.",
  organization_change: "It is the Organization change log, written by the triggers on organization-owned tables.",
} satisfies Record<string, string>;

let harness: GithubPostgresTestHarness;

beforeAll(async () => {
  harness = await startGithubPostgresTestHarness();
}, 60_000);

afterAll(async () => {
  await harness?.stop();
});

it("classifies every table as organization-owned or excluded with a reason", async () => {
  const tables = await harness.pool.query<{ relname: string }>(`
    select relname from pg_class
    where relnamespace = 'public'::regnamespace and relkind in ('r', 'p')
    order by relname
  `);
  const classified = [...Object.keys(organizationOwned), ...Object.keys(notOrganizationOwned)];
  expect(new Set(classified).size).toBe(classified.length);
  expect(tables.rows.map((row) => row.relname)).toEqual(classified.sort());
});

it("stores a non-null Organization on every organization-owned table", async () => {
  const columns = await harness.pool.query<{ table_name: string; column_name: string }>(`
    select table_name, column_name from information_schema.columns
    where table_schema = 'public' and is_nullable = 'NO'
      and (table_name, column_name) in (select * from unnest($1::text[], $2::text[]))
  `, [Object.keys(organizationOwned), Object.values(organizationOwned)]);
  expect(Object.fromEntries(columns.rows.map((row) => [row.table_name, row.column_name])))
    .toEqual(organizationOwned);
});
