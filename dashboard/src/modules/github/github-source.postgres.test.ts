import { randomUUID } from "node:crypto";
import { expect, it } from "vitest";
import { getCachedGithubRepositoryForOrganization } from "./github.repository";
import { startGithubPostgresTestHarness } from "./github-ingestion.postgres-test-harness";

it("source access requires the repository's installation to belong to a current Organization member", async () => {
  const harness = await startGithubPostgresTestHarness();
  try {
    const user = randomUUID(); const organization = randomUUID();
    await harness.pool.query('insert into "user"(id,email,name) values($1,$2,$3)', [user, "source@example.test", "Source"]);
    await harness.pool.query('insert into organization(id,name,slug) values($1,$2,$3)', [organization, "Source", "source"]);
    await harness.pool.query('insert into member(user_id,organization_id) values($1,$2)', [user, organization]);
    await harness.pool.query("insert into github_installation(user_id,installation_id,account_login,account_type) values($1,17,'owner','User')", [user]);
    await harness.pool.query("insert into github_repository_cache(user_id,installation_id,repository_id,name,full_name,default_branch,private,html_url,repo_updated_at) values($1,17,42,'repo','owner/repo','main',true,'https://github.com/owner/repo',now())", [user]);
    const input = { organizationId: organization, installationId: 17, repositoryId: 42 };
    expect(await harness.runEffect(getCachedGithubRepositoryForOrganization(input))).toEqual({ fullName: "owner/repo" });
    expect(await harness.runEffect(getCachedGithubRepositoryForOrganization({ ...input, organizationId: randomUUID() }))).toBeNull();
    expect(await harness.runEffect(getCachedGithubRepositoryForOrganization({ ...input, installationId: 18 }))).toBeNull();
    await harness.pool.query('delete from member where user_id=$1', [user]);
    expect(await harness.runEffect(getCachedGithubRepositoryForOrganization(input))).toBeNull();
  } finally { await harness.stop(); }
});
