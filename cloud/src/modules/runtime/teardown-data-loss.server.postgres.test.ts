import type { MachineId, ObservedDataLoss } from "@ployz/sdk";
import { Effect } from "effect";
import { Inngest } from "inngest";
import {
  afterAll,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
} from "vitest";
import { asTestDouble } from "#/lib/test-double";
import {
  type GithubPostgresTestHarness,
  startGithubPostgresTestHarness,
} from "#/modules/github/github-ingestion.postgres-test-harness";
import {
  OrganizationRuntime,
  type OrganizationRuntimeService,
} from "#/modules/runtime/organization-runtime.server";
import { InngestClient } from "#/modules/inngest/client";
import type { PloyzSession } from "#/modules/runtime/ployz.server";
import {
  confirmTeardown,
  loadTeardownDataLoss,
} from "#/modules/runtime/teardown.server";
import { Validation } from "#/server/public-error";

const organizationId = "00000000-0000-4000-8000-000000000801";
const userId = "00000000-0000-4000-8000-000000000802";
const projectId = "00000000-0000-4000-8000-000000000803";
const environmentId = "00000000-0000-4000-8000-000000000804";
const serviceLineageId = "00000000-0000-4000-8000-000000000805";
const serviceId = "00000000-0000-4000-8000-000000000806";
const resourceLineageId = "00000000-0000-4000-8000-000000000807";
const resourceId = "00000000-0000-4000-8000-000000000808";

const projectVolume = {
  kind: "docker_volume" as const,
  id: {
    machine_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" as MachineId,
    name: "project-data",
  },
};
const clusterOnlyVolume = {
  kind: "docker_volume" as const,
  id: {
    machine_id: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" as MachineId,
    name: "outside-cloud-project",
  },
};

function connectedRuntime(session: PloyzSession): OrganizationRuntimeService {
  return {
    open: () =>
      Effect.succeed({
        status: "connected" as const,
        connected: session,
      }),
  };
}

describe("teardown Data Loss observation", () => {
  let harness: GithubPostgresTestHarness;

  beforeAll(async () => {
    harness = await startGithubPostgresTestHarness();
  }, 60_000);

  afterAll(async () => {
    await harness.stop();
  });

  beforeEach(async () => {
    await harness.pool.query(`
      truncate table organization, "user" cascade;
      insert into organization (id, name, slug)
      values ('${organizationId}', 'Acme', 'acme');
      insert into "user" (id, email, name)
      values ('${userId}', 'teardown@example.com', 'Owner');
      insert into member (id, organization_id, user_id, role, created_at)
      values (gen_random_uuid(), '${organizationId}', '${userId}', 'owner', now());
      insert into project (id, organization_id, name, slug)
      values ('${projectId}', '${organizationId}', 'App', 'app');
      insert into environment (id, project_id, organization_id, name, namespace)
      values ('${environmentId}', '${projectId}', '${organizationId}', 'Production', 'app-production');
      insert into service_lineage (id, project_id, canonical_name, canonical_slug)
      values ('${serviceLineageId}', '${projectId}', 'API', 'api');
      insert into service (
        id, organization_id, project_id, environment_id, lineage_id, name, slug,
        source_type, source_config, private_dns
      ) values (
        '${serviceId}', '${organizationId}', '${projectId}', '${environmentId}',
        '${serviceLineageId}', 'API', 'api', 'empty',
        '{"version":1,"type":"empty","rootDir":"/"}', 'api'
      );
      insert into resource_lineage (
        id, organization_id, project_id, canonical_name, canonical_slug
      ) values (
        '${resourceLineageId}', '${organizationId}', '${projectId}', 'Data', 'data'
      );
      insert into environment_resource (
        id, organization_id, project_id, environment_id, lineage_id,
        implementation_type, name, slug
      ) values (
        '${resourceId}', '${organizationId}', '${projectId}', '${environmentId}',
        '${resourceLineageId}', 'volume', 'Data', 'data'
      );
    `);
  });

  it("uses Rust's project observation and retains Cloud rows", async () => {
    const calls: unknown[] = [];
    const observed: ObservedDataLoss = { data_loss: [projectVolume] };
    const runtime = connectedRuntime(
      asTestDouble<PloyzSession>()({
        dataLossIfProjectDestroyed: (
          ...args: Parameters<PloyzSession["dataLossIfProjectDestroyed"]>
        ) => {
          calls.push(args);
          return Effect.succeed(observed);
        },
      }),
    );

    const dataLoss = await harness.runEffect(
      Effect.scoped(
        loadTeardownDataLoss(
          { userId },
          { organizationSlug: "acme", scope: "project", projectSlug: "app" },
        ).pipe(Effect.provideService(OrganizationRuntime, runtime)),
      ),
    );

    expect(calls).toEqual([["app-production", true]]);
    expect(dataLoss).toEqual({
      rust: [projectVolume],
      cloud: [
        { kind: "environment", name: "acme/app/Production" },
        { kind: "service", name: "acme/app/Production/API" },
        { kind: "volume", name: "Data" },
        { kind: "project", name: "acme/app" },
      ],
    });
  });

  it("uses the full Rust cluster observation for organization teardown", async () => {
    const calls: string[] = [];
    const observed: ObservedDataLoss = { data_loss: [clusterOnlyVolume] };
    const runtime = connectedRuntime(
      asTestDouble<PloyzSession>()({
        dataLossIfProjectDestroyed: () =>
          Effect.die("organization teardown must not enumerate Cloud projects"),
        dataLossIfClusterDestroyed: () => {
          calls.push("cluster");
          return Effect.succeed(observed);
        },
      }),
    );

    const dataLoss = await harness.runEffect(
      Effect.scoped(
        loadTeardownDataLoss(
          { userId },
          { organizationSlug: "acme", scope: "organization" },
        ).pipe(Effect.provideService(OrganizationRuntime, runtime)),
      ),
    );

    expect(calls).toEqual(["cluster"]);
    expect(dataLoss.rust).toEqual([clusterOnlyVolume]);
    expect(dataLoss.cloud).toContainEqual({
      kind: "organization",
      name: "acme",
    });
  });

  it("refuses environment teardown when the runtime is unreachable", async () => {
    const runtime = asTestDouble<OrganizationRuntimeService>()({
      open: () => Effect.succeed({ status: "unreachable" as const, error: null }),
    });

    const failure = await harness.runEffect(
      Effect.scoped(
        loadTeardownDataLoss(
          { userId },
          { organizationSlug: "acme", scope: "environment", environmentId },
        ).pipe(Effect.provideService(OrganizationRuntime, runtime), Effect.flip),
      ),
    );

    expect(failure).toBeInstanceOf(Validation);
  });

  it("persists no Runtime project authority when confirming without a connection", async () => {
    const runtime = asTestDouble<OrganizationRuntimeService>()({
      open: () => Effect.succeed({ status: "no_connection" as const }),
    });
    const sent: unknown[] = [];
    const inngest = new Inngest({ id: "teardown-no-connection-confirm" });
    inngest.send = async (event) => {
      sent.push(event);
      return { ids: [] };
    };

    const attempt = await harness.runEffect(
      Effect.scoped(
        confirmTeardown(
          { userId },
          {
            organizationSlug: "acme",
            scope: "environment",
            environmentId,
            identities: [],
          },
        ).pipe(
          Effect.provideService(OrganizationRuntime, runtime),
          Effect.provideService(InngestClient, inngest),
        ),
      ),
    );

    expect(attempt.targets.destroyRuntimeProjects).toBe(false);
    expect(sent).toHaveLength(1);
    const rows = await harness.pool.query<{ authorized: string | null }>(
      `select targets ->> 'destroyRuntimeProjects' as authorized
       from teardown_attempt where id = $1`,
      [attempt.id],
    );
    expect(rows.rows).toEqual([{ authorized: "false" }]);
  });

  it("persists Runtime project authority when confirmation is connected", async () => {
    const runtime = connectedRuntime(asTestDouble<PloyzSession>()({}));
    const inngest = new Inngest({ id: "teardown-connected-confirm" });
    inngest.send = async () => ({ ids: [] });

    const attempt = await harness.runEffect(
      Effect.scoped(
        confirmTeardown(
          { userId },
          {
            organizationSlug: "acme",
            scope: "environment",
            environmentId,
            identities: [],
          },
        ).pipe(
          Effect.provideService(OrganizationRuntime, runtime),
          Effect.provideService(InngestClient, inngest),
        ),
      ),
    );

    expect(attempt.targets.destroyRuntimeProjects).toBe(true);
    const rows = await harness.pool.query<{ authorized: string | null }>(
      `select targets ->> 'destroyRuntimeProjects' as authorized
       from teardown_attempt where id = $1`,
      [attempt.id],
    );
    expect(rows.rows).toEqual([{ authorized: "true" }]);
  });
});
