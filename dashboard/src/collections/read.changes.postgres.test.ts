import { randomUUID } from "node:crypto";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { readChangeWindow } from "./changes.server";
import { readCollection } from "./read.server";
import type { CollectionRead } from "./read.contract";
import {
  type GithubPostgresTestHarness,
  startGithubPostgresTestHarness,
} from "#/modules/github/github-ingestion.postgres-test-harness";

type ServiceRow = { id: string; name: string; organizationId: string };
type Organization = { id: string; slug: string; userId: string; projectId: string; environmentId: string };

describe("incremental Service reads from the Organization change log", () => {
  let harness: GithubPostgresTestHarness;
  let alpha: Organization;
  let beta: Organization;

  const sql = (text: string, values: unknown[] = []) => harness.pool.query(text, values);

  async function createOrganization(slug: string): Promise<Organization> {
    const organization = { id: randomUUID(), slug, userId: randomUUID(), projectId: randomUUID(), environmentId: randomUUID() };
    await sql("insert into organization (id, name, slug) values ($1, $2, $2)", [organization.id, slug]);
    await sql("insert into \"user\" (id, email, name) values ($1, $2, $2)", [organization.userId, `${slug}@example.test`]);
    await sql("insert into member (user_id, organization_id, role) values ($1, $2, 'owner')", [organization.userId, organization.id]);
    await sql("insert into project (id, organization_id, name, slug) values ($1, $2, 'api', 'api')", [organization.projectId, organization.id]);
    await sql(
      "insert into environment (id, organization_id, project_id, name, namespace, intent) values ($1, $2, $3, 'production', 'production', '{}')",
      [organization.environmentId, organization.id, organization.projectId],
    );
    return organization;
  }

  /** One statement, so one change log row per Organization. */
  async function createServices(organization: Organization, names: string[]) {
    const result = await harness.pool.query<{ id: string }>(`
      with lineage as (
        insert into service_lineage (organization_id, project_id, canonical_name, canonical_slug)
        select $3, $1, name, name from unnest($2::text[]) as name returning id, canonical_name
      )
      insert into service (organization_id, project_id, environment_id, lineage_id, name)
      select $3, $1, $4, id, canonical_name from lineage returning id
    `, [organization.projectId, names, organization.id, organization.environmentId]);
    return result.rows.map((row) => row.id);
  }

  function read(organization: Organization, since?: string) {
    return harness.runEffect(readCollection({ userId: organization.userId }, {
      table: "service", userId: organization.userId, organizationSlug: organization.slug, since,
    })) as Promise<CollectionRead<ServiceRow>>;
  }

  async function cursorNow(organization: Organization) {
    const snapshot = await read(organization);
    if (snapshot.cursor === null) throw new Error("service reads carry a change cursor");
    return snapshot.cursor;
  }

  beforeAll(async () => {
    harness = await startGithubPostgresTestHarness();
    alpha = await createOrganization(`alpha-${randomUUID().slice(0, 8)}`);
    beta = await createOrganization(`beta-${randomUUID().slice(0, 8)}`);
  }, 60_000);

  afterAll(async () => {
    await harness?.stop();
  });

  it("returns an Organization's own changes since the cursor and never another's", async () => {
    const [alphaService] = await createServices(alpha, [`web-${randomUUID().slice(0, 8)}`]);
    await createServices(beta, [`web-${randomUUID().slice(0, 8)}`]);
    const since = await cursorNow(alpha);

    await sql("update service set name = 'renamed' where id = $1", [alphaService]);

    const alphaChanges = await read(alpha, since);
    expect(alphaChanges).toMatchObject({ full: false, deleted: [], rows: [{ id: alphaService, name: "renamed" }] });
    const betaChanges = await read(beta, since);
    expect(betaChanges).toMatchObject({ full: false, deleted: [], rows: [] });
    // Nothing changed, yet the cursor still moves forward.
    expect(BigInt(betaChanges.cursor ?? "0")).toBeGreaterThan(BigInt(since));
    // The change stream's reader sees no tables for the other Organization.
    expect(await harness.runEffect(readChangeWindow({ organizationId: beta.id, since })))
      .toMatchObject({ sourceTables: [], changed: [], deleted: [] });
    const quiet = await read(alpha, alphaChanges.cursor ?? undefined);
    expect(quiet).toMatchObject({ full: false, deleted: [], rows: [] });
  });

  it("returns a deleted Service as a deleted id", async () => {
    const [service] = await createServices(alpha, [`worker-${randomUUID().slice(0, 8)}`]);
    const since = await cursorNow(alpha);

    await sql("delete from service where id = $1", [service]);

    expect(await read(alpha, since)).toMatchObject({ full: false, rows: [], deleted: [service] });
    expect(await read(beta, since)).toMatchObject({ full: false, rows: [], deleted: [] });
  });

  it("falls back to a full read when one statement touches more than 100 Services", async () => {
    const since = await cursorNow(alpha);

    const created = await createServices(alpha, Array.from({ length: 101 }, (_, index) => `bulk-${index}-${randomUUID().slice(0, 8)}`));

    const changes = await read(alpha, since);
    expect(changes.full).toBe(true);
    expect(changes.rows.map((row) => row.id)).toEqual(expect.arrayContaining(created));
    expect(changes.rows.every((row) => row.organizationId === alpha.id)).toBe(true);
  });

  it("holds back changes until every older transaction finishes, and skips none", async () => {
    const [first, second, third] = await createServices(alpha, ["a", "b", "c"].map((name) => `${name}-${randomUUID().slice(0, 8)}`));
    const since = await cursorNow(alpha);
    const older = await harness.pool.connect();
    const newer = await harness.pool.connect();
    try {
      // `older` takes its transaction id first but logs its Service change last.
      await older.query("begin");
      await older.query("select pg_current_xact_id()");
      await newer.query("begin");
      await newer.query("update service set name = 'second' where id = $1", [second]);
      await older.query("update service set name = 'first' where id = $1", [first]);
      await older.query("commit");
      // A later transaction commits while `newer` still holds an earlier change.
      await sql("update service set name = 'third' where id = $1", [third]);

      const held = await read(alpha, since);
      expect(held.rows.map((row) => row.name)).toEqual(["first"]);

      await newer.query("commit");
      const released = await read(alpha, held.cursor ?? undefined);
      expect(released.rows.map((row) => row.name).sort()).toEqual(["second", "third"]);
    } finally {
      await newer.query("rollback").catch(() => undefined);
      older.release();
      newer.release();
    }
  });
});
