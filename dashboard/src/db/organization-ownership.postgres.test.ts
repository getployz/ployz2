import { afterAll, beforeAll, expect, it } from "vitest";
import { changeSources, collectionSources } from "#/collections/change-sources";
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

it("logs every change to an organization-owned table under its Organization and the key its collections share", async () => {
  const triggers = await harness.pool.query<{ table_name: string; event: string; tgargs: Buffer }>(`
    select c.relname as table_name, t.tgname as event, t.tgargs
    from pg_trigger t join pg_class c on c.oid = t.tgrelid join pg_proc p on p.oid = t.tgfoid
    where p.proname = 'organization_change_log'
  `);
  const row = (table: string, event: string, args: string[]) => `${table} ${event}(${args.join(", ")})`;
  // Arguments are the Organization column, then the one key every collection fed by the table shares.
  const expected = Object.entries(organizationOwned).flatMap(([table, organizationColumn]) =>
    ["insert", "update", "delete"].map((event) =>
      row(table, `organization_change_${event}`, [organizationColumn, ...new Map<string, readonly string[]>(Object.entries(changeSources)).get(table) ?? []])));
  expect(triggers.rows.map((trigger) =>
    row(trigger.table_name, trigger.event, trigger.tgargs.toString("utf8").split("\0").filter(Boolean))).sort())
    .toEqual(expected.sort());
});

it("keys every source feeding a collection by its key table's key", async () => {
  const foreignKeys = await harness.pool.query<{ source: string; columns: string[]; target: string; target_columns: string[] }>(`
    select c.conrelid::regclass::text as source, c.confrelid::regclass::text as target,
      array(select a.attname::text from unnest(c.conkey) with ordinality k(n, i)
        join pg_attribute a on a.attrelid = c.conrelid and a.attnum = k.n order by k.i) as columns,
      array(select a.attname::text from unnest(c.confkey) with ordinality k(n, i)
        join pg_attribute a on a.attrelid = c.confrelid and a.attnum = k.n order by k.i) as target_columns
    from pg_constraint c where c.contype = 'f'
  `);
  const references = new Set(foreignKeys.rows.map((row) => `${row.source}(${row.columns.join(", ")}) -> ${row.target}(${row.target_columns.join(", ")})`));
  // A change to any source names the collection rows it affects only if it logs their key.
  const required = Object.values(collectionSources).flatMap(([keyTable, ...others]) =>
    others.map((source) => `${source}(${changeSources[source].join(", ")}) -> ${keyTable}(${changeSources[keyTable].join(", ")})`));
  expect(required.length).toBeGreaterThan(0);
  expect(required.filter((reference) => !references.has(reference))).toEqual([]);
});
