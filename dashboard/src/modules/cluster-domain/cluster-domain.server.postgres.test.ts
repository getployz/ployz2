import { ConfigProvider, Effect, Layer } from "effect";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { readCollection } from "#/collections/read.server";
import { publishClusterDomainNow, reserveClusterDomain } from "#/modules/cluster-domain/cluster-domain.server";
import { type FakeHostedDns, startFakeHostedDns } from "#/modules/cluster-domain/hosted-dns.test-fixture";
import {
  type GithubPostgresTestHarness,
  startGithubPostgresTestHarness,
} from "#/modules/github/github-ingestion.postgres-test-harness";
import { AppConfig } from "#/server/config.server";
import type { Database, ReportingDatabase } from "#/server/database.server";
import { makeSecretEncryption, SecretEncryption } from "#/utils/encrypted-secret.server";

const organizationId = "00000000-0000-4000-8000-000000000a01";
const userId = "00000000-0000-4000-8000-000000000a02";
const encryption = makeSecretEncryption("cluster-domain-test-encryption-secret");

describe("Organization Cluster Domain", () => {
  let harness: GithubPostgresTestHarness;
  let hostedDns: FakeHostedDns;

  function run<A, E>(
    operation: Effect.Effect<A, E, Database | ReportingDatabase | AppConfig | SecretEncryption>,
    env: Record<string, string> = {},
  ) {
    const config = AppConfig.layer.pipe(Layer.provide(ConfigProvider.layer(ConfigProvider.fromEnv({
      env: {
        DATABASE_URL: harness.databaseUrl,
        APP_URL: "https://cloud.example.test",
        BETTER_AUTH_SECRET: "better-auth-secret",
        GITHUB_CLIENT_ID: "github-client-id",
        GITHUB_CLIENT_SECRET: "github-client-secret",
        APP_ENCRYPTION_SECRET: "app-encryption-secret-at-least-32-characters",
        PLOYZ_HOSTED_DNS_URL: hostedDns.url,
        ...env,
      },
    }))));
    return harness.runEffect(operation.pipe(Effect.provideService(SecretEncryption, encryption), Effect.provide(config)));
  }

  const rows = () => harness.pool.query<{ endpoint: string; name: string; encrypted_token: Parameters<typeof encryption.decrypt>[0] }>(
    "select endpoint, name, encrypted_token from organization_cluster_domain");

  beforeAll(async () => {
    [harness, hostedDns] = await Promise.all([startGithubPostgresTestHarness(), startFakeHostedDns()]);
  }, 60_000);

  afterAll(async () => {
    await Promise.all([harness?.stop(), hostedDns?.close()]);
  });

  beforeEach(async () => {
    hostedDns.requests.length = 0;
    hostedDns.state.failWith = null;
    await harness.pool.query(`
      truncate table organization, "user" cascade;
      insert into organization (id, name, slug) values ('${organizationId}', 'Acme', 'acme');
      insert into "user" (id, email, name) values ('${userId}', 'domain@example.com', 'Owner');
      insert into member (id, organization_id, user_id, role, created_at)
      values (gen_random_uuid(), '${organizationId}', '${userId}', 'owner', now());
    `);
  });

  it("reserves once with the Organization slug and stores the granted name, endpoint and encrypted token", async () => {
    const reserved = await run(reserveClusterDomain(organizationId));
    const again = await run(reserveClusterDomain(organizationId));

    expect(hostedDns.requests).toEqual([{ method: "POST", path: "/domains", authorization: null, body: { preferred: "acme" } }]);
    expect(again).toEqual(reserved);
    const [row] = (await rows()).rows;
    expect(row).toMatchObject({ endpoint: hostedDns.url, name: "acme.ployz.test" });
    expect(row && encryption.decrypt(row.encrypted_token)).toBe("token-1");
  });

  it("sends the mint key as the bearer token only when it is configured", async () => {
    await run(reserveClusterDomain(organizationId), { PLOYZ_HOSTED_DNS_MINT_KEY: "mint-key" });
    await harness.pool.query("delete from organization_cluster_domain");
    await run(reserveClusterDomain(organizationId));
    expect(hostedDns.requests.map((request) => request.authorization)).toEqual(["Bearer mint-key", null]);
  });

  it("stores nothing when Hosted DNS fails", async () => {
    hostedDns.state.failWith = 503;
    const failed = await run(reserveClusterDomain(organizationId).pipe(Effect.flip));
    expect(failed).toMatchObject({ _tag: "HostedDnsError", status: 503 });
    expect((await rows()).rows).toEqual([]);
  });

  it("shows the name in the Org Store without the token", async () => {
    const read = () => run(readCollection({ userId }, { table: "organization_cluster_domain", organizationSlug: "acme", userId }));
    expect((await read()).rows).toEqual([]);
    await run(reserveClusterDomain(organizationId));
    const [row] = (await read()).rows;
    expect(row).toMatchObject({ id: organizationId, name: "acme.ployz.test", recordsSyncedAt: null, published: [], certificateNotAfter: null });
    expect(JSON.stringify(row)).not.toContain("token");
  });

  it("Publish now reserves a missing name and reports an unreachable Hosted DNS", async () => {
    hostedDns.state.failWith = 500;
    expect(await run(publishClusterDomainNow({ userId }, { organizationSlug: "acme" }).pipe(Effect.flip)))
      .toMatchObject({ _tag: "Conflict" });
    hostedDns.state.failWith = null;
    expect(await run(publishClusterDomainNow({ userId }, { organizationSlug: "acme" }))).toEqual({ name: "acme.ployz.test" });
    expect(await run(publishClusterDomainNow({ userId: organizationId }, { organizationSlug: "acme" }).pipe(Effect.flip)))
      .toMatchObject({ _tag: "NotFound" });
  });
});
