import { randomUUID } from "node:crypto";
import { parseServiceConfig } from "@ployz/sdk/config";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { desc, eq, sql } from "drizzle-orm";
import { Effect, Exit } from "effect";
import { Inngest } from "inngest";
import * as schema from "#/db/schema";
import {
  type GithubPostgresTestHarness,
  startGithubPostgresTestHarness,
} from "#/modules/github/github-ingestion.postgres-test-harness";
import { InngestClient } from "#/modules/inngest/client";
import { loadCurrentEnvironmentSnapshotProjection } from "#/modules/environment-design/working-state-repository.server";
import { fingerprintReviewedEnvironmentWorkingState } from "#/modules/environment-design/working-state-review";
import { markDeploymentStatus } from "#/modules/deployments/runtime-repository.server";
import {
  createEnvironmentDeploymentSnapshot,
  retryEnvironmentDeployment,
} from "./deployment-operations.server";
import {
  makeSecretEncryption,
  SecretEncryption,
} from "#/utils/encrypted-secret.server";

const encryption = makeSecretEncryption("test-encryption-secret");
const organizationId = "00000000-0000-4000-8000-000000000901";
const userId = "00000000-0000-4000-8000-000000000902";
const projectId = "00000000-0000-4000-8000-000000000903";
const environmentId = "00000000-0000-4000-8000-000000000904";
const lineageId = "00000000-0000-4000-8000-000000000905";
const serviceId = "00000000-0000-4000-8000-000000000906";
const actor = { userId };
const slugs = {
  organizationSlug: "command",
  projectSlug: "cloud",
  environmentSlug: "production",
} as const;

describe("Cloud Save and Deploy command", () => {
  let harness: GithubPostgresTestHarness;
  const inngest = new Inngest({ id: "cloud-command-test" });
  const sent: unknown[] = [];
  vi.spyOn(inngest, "send").mockImplementation(async (event) => {
    sent.push(event);
    return { ids: [] };
  });

  beforeAll(async () => {
    harness = await startGithubPostgresTestHarness();
  }, 60_000);

  afterAll(async () => {
    await harness?.stop();
  });

  beforeEach(async () => {
    sent.length = 0;
    const { env: _env, mounts: _mounts, variableGroupAttachments: _variableGroupAttachments, ...config } =
      parseServiceConfig({
        version: 2,
        name: "API",
        source: { version: 1, type: "empty", rootDir: "/" },
        healthcheck: { type: "none" },
        restartPolicy: "unless-stopped",
        privateDns: "api",
      });
    const intent = {
      version: 1,
      environmentSlug: "production",
      variableGroups: [],
      volumes: [],
      services: [{
        id: serviceId,
        lineageId,
        slug: "api",
        config,
        encryptedRegistryUsername: null,
        encryptedRegistrySecret: null,
        variables: [],
        variableGroupAttachments: [],
        volumeAttachments: [],
      }],
    };
    await harness.pool.query(`
      truncate table organization, "user" cascade;
      insert into organization (id, name, slug)
      values ('${organizationId}', 'Command', 'command');
      insert into "user" (id, email, name)
      values ('${userId}', 'owner@example.com', 'Owner');
      insert into member (id, organization_id, user_id, role, created_at)
      values (gen_random_uuid(), '${organizationId}', '${userId}', 'owner', now());
      insert into project (id, organization_id, name, slug)
      values ('${projectId}', '${organizationId}', 'Cloud', 'cloud');
      insert into environment (id, project_id, organization_id, name, namespace, intent)
      values ('${environmentId}', '${projectId}', '${organizationId}', 'Production', 'production', '${JSON.stringify(intent)}');
      insert into service_lineage (id, project_id, canonical_name, canonical_slug)
      values ('${lineageId}', '${projectId}', 'API', 'api');
      insert into service (id, project_id, environment_id, organization_id, lineage_id)
      values ('${serviceId}', '${projectId}', '${environmentId}', '${organizationId}', '${lineageId}');
    `);
  });

  async function review() {
    const [latest] = await harness.db
      .select({ id: schema.environmentSavedStateSnapshot.id })
      .from(schema.environmentSavedStateSnapshot)
      .where(eq(schema.environmentSavedStateSnapshot.environmentId, environmentId))
      .orderBy(
        desc(schema.environmentSavedStateSnapshot.createdAt),
        desc(schema.environmentSavedStateSnapshot.id),
      )
      .limit(1);
    return {
      savedStateBasis: latest
        ? { kind: "saved_revision" as const, savedStateSnapshotId: latest.id }
        : { kind: "no_saved_state" as const },
      reviewedWorkingStateFingerprint: await harness.runTransaction(() =>
        loadCurrentEnvironmentSnapshotProjection(environmentId).pipe(
          Effect.map(fingerprintReviewedEnvironmentWorkingState),
        ),
      ),
      destructiveServiceIds: [] as string[],
      destructiveVolumeReviews: [],
    };
  }

  function submit(input: {
    deploy: boolean;
    message?: string;
    destructiveServiceIds?: string[];
  }) {
    return review().then((publication) =>
      harness.runEffect(
        createEnvironmentDeploymentSnapshot(actor, {
          ...slugs,
          message: input.message ?? null,
          deploy: input.deploy,
          ...publication,
          destructiveServiceIds: input.destructiveServiceIds ?? publication.destructiveServiceIds,
        }).pipe(
          Effect.provideService(InngestClient, inngest),
          Effect.provideService(SecretEncryption, encryption),
          Effect.exit,
        ),
      ),
    );
  }

  it("saves without a deployment or workflow event", async () => {
    const exit = await submit({ deploy: false, message: "Prepare" });
    expect(Exit.isSuccess(exit)).toBe(true);
    if (Exit.isSuccess(exit)) expect(exit.value).toEqual({ state: "saved" });
    expect(await harness.db.select().from(schema.environmentDeployment)).toEqual([]);
    expect(sent).toEqual([]);
    expect(await harness.db.select().from(schema.environmentSavedStateSnapshot)).toHaveLength(1);
  });

  it("deploys the exact reviewed Saved revision", async () => {
    const exit = await submit({ deploy: true, message: "Ship" });
    expect(Exit.isSuccess(exit)).toBe(true);
    if (Exit.isSuccess(exit)) expect(exit.value).toEqual({ state: "deployment_queued" });
    const [saved] = await harness.db.select().from(schema.environmentSavedStateSnapshot);
    const [attempt] = await harness.db.select().from(schema.environmentDeployment);
    expect(attempt).toMatchObject({
      status: "queued",
      savedStateSnapshotId: saved?.id,
      triggerOrigin: { origin: "manual", actorId: userId },
    });
    expect(sent).toHaveLength(1);
  });

  it("rejects Save and Deploy when a deployed Service is removed without that id", async () => {
    const deployed = await submit({ deploy: true, message: "Baseline" });
    expect(Exit.isSuccess(deployed)).toBe(true);
    const [attempt] = await harness.db.select().from(schema.environmentDeployment);
    if (!attempt) throw new Error("expected a queued attempt");
    await harness.runEffect(
      markDeploymentStatus({
        environmentDeploymentId: attempt.id,
        status: "applied",
      }).pipe(
        Effect.provideService(SecretEncryption, encryption),
        Effect.provideService(InngestClient, inngest),
      ),
    );
    await harness.db
      .update(schema.environment)
      .set({
        intent: {
          version: 1,
          environmentSlug: "production",
          services: [],
          variableGroups: [],
          volumes: [],
        },
        revision: randomUUID(),
      })
      .where(eq(schema.environment.id, environmentId));

    const saveExit = await submit({ deploy: false, message: "Unreviewed save" });
    const deployExit = await submit({ deploy: true, message: "Unreviewed deploy" });
    expect(Exit.isFailure(saveExit)).toBe(true);
    expect(Exit.isFailure(deployExit)).toBe(true);
    expect(await harness.db.select().from(schema.environmentSavedStateSnapshot)).toHaveLength(1);
    expect(await harness.db.select().from(schema.environmentDeployment)).toHaveLength(1);
  });

  it("refuses a second manual Deploy while one attempt is queued", async () => {
    const first = await submit({ deploy: true, message: "First" });
    expect(Exit.isSuccess(first)).toBe(true);
    const [queued] = await harness.db.select().from(schema.environmentDeployment);
    await harness.db
      .update(schema.environment)
      .set({
        intent: sql`jsonb_set(${schema.environment.intent}, '{services,0,config,name}', to_jsonb(${"Changed"}::text))`,
        revision: randomUUID(),
      })
      .where(eq(schema.environment.id, environmentId));

    const second = await submit({ deploy: true, message: "Second" });
    expect(Exit.isFailure(second)).toBe(true);
    const attempts = await harness.db.select().from(schema.environmentDeployment);
    const saved = await harness.db.select().from(schema.environmentSavedStateSnapshot);
    expect(attempts).toHaveLength(1);
    expect(attempts[0]?.id).toBe(queued?.id);
    expect(attempts[0]?.savedStateSnapshotId).toBe(queued?.savedStateSnapshotId);
    expect(saved).toHaveLength(1);
  });

  it("retries a failed attempt on the same Saved revision", async () => {
    const deployed = await submit({ deploy: true, message: "Ship" });
    expect(Exit.isSuccess(deployed)).toBe(true);
    const [attempt] = await harness.db.select().from(schema.environmentDeployment);
    if (!attempt) throw new Error("expected a queued attempt");
    await harness.db
      .update(schema.environmentDeployment)
      .set({
        status: "failed",
        failureCode: "runtime_failed",
        failureMessage: "runtime failed",
        finishedAt: new Date(),
      })
      .where(eq(schema.environmentDeployment.id, attempt.id));

    const retried = await harness.runEffect(
      retryEnvironmentDeployment(actor, {
        ...slugs,
        failedDeploymentId: attempt.id,
      }).pipe(
        Effect.provideService(InngestClient, inngest),
        Effect.provideService(SecretEncryption, encryption),
        Effect.exit,
      ),
    );
    expect(Exit.isSuccess(retried)).toBe(true);
    const rows = await harness.db
      .select()
      .from(schema.environmentDeployment)
      .orderBy(desc(schema.environmentDeployment.createdAt));
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({
      status: "queued",
      savedStateSnapshotId: attempt.savedStateSnapshotId,
      retryOfDeploymentId: attempt.id,
      triggerOrigin: { origin: "manual", actorId: userId },
    });
  });

  it("refuses retry when any volume removal row exists", async () => {
    const deployed = await submit({ deploy: true, message: "Ship" });
    expect(Exit.isSuccess(deployed)).toBe(true);
    const [attempt] = await harness.db.select().from(schema.environmentDeployment);
    if (!attempt) throw new Error("expected a queued attempt");
    await harness.db
      .update(schema.environmentDeployment)
      .set({
        status: "failed",
        failureCode: "inngest_dispatch_failed",
        failureMessage: "Cloud could not dispatch the deployment workflow.",
        finishedAt: new Date(),
      })
      .where(eq(schema.environmentDeployment.id, attempt.id));
    await harness.db.insert(schema.volumeRemoveAttempt).values({
      organizationId,
      requestedByUserId: userId,
      environmentId,
      environmentDeploymentId: attempt.id,
      environmentResourceId: randomUUID(),
      volumes: [{ machine_id: "a".repeat(32), name: "vol-disk" }],
      status: "failed",
      failureMessage:
        "Deployment failed before the approved volume removal was submitted.",
      terminalAt: new Date(),
    });

    const retried = await harness.runEffect(
      retryEnvironmentDeployment(actor, {
        ...slugs,
        failedDeploymentId: attempt.id,
      }).pipe(
        Effect.provideService(InngestClient, inngest),
        Effect.provideService(SecretEncryption, encryption),
        Effect.exit,
      ),
    );
    expect(Exit.isFailure(retried)).toBe(true);
    expect(await harness.db.select().from(schema.environmentDeployment)).toHaveLength(
      1,
    );
  });
});
