import { randomUUID } from "node:crypto";
import type { MachineId } from "@ployz/sdk";
import { parseServiceConfig } from "@ployz/sdk/config";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { asc, desc, eq, sql } from "drizzle-orm";
import { Effect } from "effect";
import { Inngest } from "inngest";
import * as schema from "#/db/schema";
import { submitReviewedPublication } from "#/modules/deployments/cloud-deployment-command.server";
import type { ReviewedPublicationInput } from "#/modules/deployments/deployment-contract";
import { ENVIRONMENT_DEPLOYMENT_DISPATCH_FAILURE_CODE } from "#/modules/deployments/dispatch.server";
import { createRetryAttempt } from "#/modules/deployments/retry-repository.server";
import { markDeploymentStatus } from "#/modules/deployments/runtime-repository.server";
import { markDeploymentCancelled } from "#/modules/deployments/runtime-cancellation.repository.server";
import { fingerprintReviewedEnvironmentWorkingState } from "#/modules/environment-design/working-state-review";
import { loadCurrentEnvironmentSnapshotProjection } from "#/modules/environment-design/working-state-repository.server";
import {
  type GithubPostgresTestHarness,
  startGithubPostgresTestHarness,
} from "#/modules/github/github-ingestion.postgres-test-harness";
import { InngestClient } from "#/modules/inngest/client";
import { OrganizationRuntime } from "#/modules/runtime/organization-runtime.server";
import { stageVolumeRemoveAttempt } from "#/modules/runtime/volume-removal.repository";
import {
  makeSecretEncryption,
  SecretEncryption,
} from "#/utils/encrypted-secret.server";

const encryption = makeSecretEncryption("test-encryption-secret");

const organizationId = "00000000-0000-4000-8000-000000000501";
const userId = "00000000-0000-4000-8000-000000000502";
const projectId = "00000000-0000-4000-8000-000000000503";
const environmentId = "00000000-0000-4000-8000-000000000504";
const lineageId = "00000000-0000-4000-8000-000000000505";
const serviceId = "00000000-0000-4000-8000-000000000506";
const volumeId = "00000000-0000-4000-8000-000000000507";
const machineId = "a".repeat(32) as MachineId;
const actor = { userId };
const slugs = {
  organizationSlug: "acceptance",
  projectSlug: "cloud",
  environmentSlug: "production",
};

type SentDispatch = { environmentDeploymentId: string; environmentId: string };

function recordingInngest(sent: SentDispatch[]) {
  const inngest = new Inngest({ id: "cloud-deployment-command-test" });
  inngest.send = async (event) => {
    const payload = Array.isArray(event) ? event[0] : event;
    if (payload !== undefined) sent.push(payload.data as SentDispatch);
    return { ids: [] };
  };
  return inngest;
}

function failingInngest() {
  const inngest = new Inngest({ id: "cloud-deployment-command-fail-test" });
  inngest.send = async () => {
    throw new Error("Inngest unavailable");
  };
  return inngest;
}

describe("Cloud deployment command", () => {
  let harness: GithubPostgresTestHarness;

  beforeAll(async () => {
    harness = await startGithubPostgresTestHarness();
  }, 60_000);

  afterAll(async () => {
    await harness?.stop();
  });

  beforeEach(async () => {
    const { env: _env, mounts: _mounts, variableGroupAttachments: _attachments, ...config } = parseServiceConfig({ version: 2, name: "API", source: { version: 1, type: "empty", rootDir: "/" }, healthcheck: { type: "none" }, restartPolicy: "unless-stopped", privateDns: "api" });
    const intent = { version: 1, environmentSlug: "production", variableGroups: [], volumes: [], services: [{
      id: serviceId, lineageId, slug: "api", config, encryptedRegistryUsername: null, encryptedRegistrySecret: null,
      variables: [], variableGroupAttachments: [], volumeAttachments: [],
    }] };
    await harness.pool.query(`
      truncate table organization, "user" cascade;
      insert into organization (id, name, slug) values ('${organizationId}', 'Acceptance', 'acceptance');
      insert into "user" (id, email, name) values ('${userId}', 'owner@example.com', 'Owner');
      insert into member (id, organization_id, user_id, role, created_at)
        values (gen_random_uuid(), '${organizationId}', '${userId}', 'owner', now());
      insert into project (id, organization_id, name, slug) values ('${projectId}', '${organizationId}', 'Cloud', 'cloud');
      insert into environment (id, project_id, organization_id, name, namespace, intent)
        values ('${environmentId}', '${projectId}', '${organizationId}', 'Production', 'production', '${JSON.stringify(intent)}');
      insert into service_lineage (id, project_id, canonical_name, canonical_slug)
        values ('${lineageId}', '${projectId}', 'API', 'api');
      insert into service (id, project_id, environment_id, organization_id, lineage_id)
        values ('${serviceId}', '${projectId}', '${environmentId}', '${organizationId}', '${lineageId}');
    `);
  });

  async function currentReview() {
    const fingerprint = await harness.runTransaction(() =>
      loadCurrentEnvironmentSnapshotProjection(environmentId).pipe(
        Effect.map(fingerprintReviewedEnvironmentWorkingState),
      ),
    );
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
      workingStateFingerprint: fingerprint,
      savedStateBasis: latest
        ? { kind: "saved_revision" as const, savedStateSnapshotId: latest.id }
        : { kind: "no_saved_state" as const },
      destructiveServiceIds: [] as string[],
      destructiveVolumeReviews: [],
    };
  }

  function submit(input: {
    intent: ReviewedPublicationInput["intent"];
    message: string;
    review: ReviewedPublicationInput["review"];
    inngest: Inngest;
  }) {
    return harness.runEffect(
      submitReviewedPublication(actor, {
        ...slugs,
        intent: input.intent,
        message: input.message,
        review: input.review,
      }).pipe(
        Effect.provideService(SecretEncryption, encryption),
        Effect.provideService(InngestClient, input.inngest),
        Effect.provideService(OrganizationRuntime, {
          cancel: () => Effect.void,
          open: () => Effect.die("the command must not open a runtime session without Volume removals"),
        }),
      ),
    );
  }

  async function deploy(message: string, inngest = recordingInngest([])) {
    const outcome = await submit({
      intent: "manual_deploy",
      message,
      review: await currentReview(),
      inngest,
    });
    if (outcome.state !== "deployment_queued") {
      throw new Error(`expected a queued deployment, got ${outcome.state}`);
    }
    return outcome.environmentDeploymentId;
  }

  function savedRows() {
    return harness.db
      .select({
        id: schema.environmentSavedStateSnapshot.id,
        actorId: schema.environmentSavedStateSnapshot.actorId,
        message: schema.environmentSavedStateSnapshot.message,
      })
      .from(schema.environmentSavedStateSnapshot)
      .where(eq(schema.environmentSavedStateSnapshot.environmentId, environmentId))
      .orderBy(asc(schema.environmentSavedStateSnapshot.createdAt));
  }

  function attemptRows() {
    return harness.db
      .select({
        id: schema.environmentDeployment.id,
        status: schema.environmentDeployment.status,
        failureCode: schema.environmentDeployment.failureCode,
        savedStateSnapshotId: schema.environmentDeployment.savedStateSnapshotId,
        retryOfDeploymentId: schema.environmentDeployment.retryOfDeploymentId,
        triggerOrigin: schema.environmentDeployment.triggerOrigin,
      })
      .from(schema.environmentDeployment)
      .where(eq(schema.environmentDeployment.environmentId, environmentId))
      .orderBy(asc(schema.environmentDeployment.createdAt));
  }

  function renameWorkingService(name: string) {
    return harness.db
      .update(schema.environment)
      .set({
        intent: sql`jsonb_set(${schema.environment.intent}, '{services,0,config,name}', to_jsonb(${name}::text))`,
        revision: randomUUID(),
      })
      .where(eq(schema.environment.id, environmentId));
  }

  async function applyAttempt(environmentDeploymentId: string) {
    await harness.runEffect(
      markDeploymentStatus({ environmentDeploymentId, status: "applied" }).pipe(
        Effect.provideService(SecretEncryption, encryption),
        Effect.provideService(InngestClient, recordingInngest([])),
      ),
    );
  }

  it("Save publishes the reviewed Working State and admits nothing", async () => {
    const sent: SentDispatch[] = [];

    const outcome = await submit({
      intent: "save",
      message: "Reviewed save",
      review: await currentReview(),
      inngest: recordingInngest(sent),
    });

    expect(outcome).toEqual({ state: "saved" });
    expect(await savedRows()).toEqual([
      { id: expect.any(String), actorId: userId, message: "Reviewed save" },
    ]);
    expect(await attemptRows()).toEqual([]);
    expect(sent).toEqual([]);
  });

  it("Deploy publishes, admits that exact Saved revision, and dispatches after commit", async () => {
    const sent: SentDispatch[] = [];

    const outcome = await submit({
      intent: "manual_deploy",
      message: "Reviewed deploy",
      review: await currentReview(),
      inngest: recordingInngest(sent),
    });

    const [saved] = await savedRows();
    expect(saved).toEqual({ id: expect.any(String), actorId: userId, message: "Reviewed deploy" });
    expect(outcome).toEqual({
      state: "deployment_queued",
      environmentDeploymentId: expect.any(String),
    });
    if (outcome.state !== "deployment_queued" || saved === undefined) return;
    expect(await attemptRows()).toEqual([
      {
        id: outcome.environmentDeploymentId,
        status: "queued",
        failureCode: null,
        savedStateSnapshotId: saved.id,
        retryOfDeploymentId: null,
        triggerOrigin: { origin: "manual", actorId: userId },
      },
    ]);
    expect(sent).toEqual([
      { environmentDeploymentId: outcome.environmentDeploymentId, environmentId },
    ]);
  });

  it("Deploy refuses an unreviewed Service removal and accepts the complete review", async () => {
    await applyAttempt(await deploy("Applied baseline"));
    await harness.db
      .update(schema.environment)
      .set({ intent: { version: 1, environmentSlug: "production", services: [], variableGroups: [], volumes: [] }, revision: randomUUID() })
      .where(eq(schema.environment.id, environmentId));
    const review = await currentReview();

    await expect(
      submit({ intent: "manual_deploy", message: "Unreviewed removal", review, inngest: recordingInngest([]) }),
    ).rejects.toMatchObject({
      _tag: "Conflict",
      message: expect.stringContaining("changed after review"),
    });
    expect(await savedRows()).toHaveLength(1);
    expect(await attemptRows()).toHaveLength(1);

    const reviewed = await submit({
      intent: "manual_deploy",
      message: "Reviewed removal",
      review: { ...review, destructiveServiceIds: [serviceId] },
      inngest: recordingInngest([]),
    });

    expect(reviewed.state).toBe("deployment_queued");
    expect(await savedRows()).toHaveLength(2);
    expect((await attemptRows()).map((row) => row.status)).toEqual(["applied", "queued"]);
  });

  it("a second manual Deploy conflicts while one is queued and admits again once it is cancelled", async () => {
    const first = await deploy("First target");
    const [firstSaved] = await savedRows();
    await renameWorkingService("Changed target");

    await expect(deploy("Second target")).rejects.toMatchObject({
      _tag: "Conflict",
      message: "An environment deployment attempt is already queued.",
    });
    expect(await savedRows()).toHaveLength(1);
    expect(await attemptRows()).toEqual([
      expect.objectContaining({
        id: first,
        status: "queued",
        savedStateSnapshotId: firstSaved?.id,
      }),
    ]);

    await harness.runEffect(
      markDeploymentCancelled(
        { deploymentId: first, beforeExecution: true },
        "Cancelled so a later Deploy can admit.",
      ),
    );
    const third = await deploy("Third target");

    expect(third).not.toBe(first);
    expect(await savedRows()).toHaveLength(2);
    expect((await attemptRows()).map((row) => [row.id, row.status])).toEqual([
      [first, "cancelled"],
      [third, "queued"],
    ]);
  });

  it("a dispatch send failure fails the attempt, keeps the Saved revision, and an ordinary retry pins it", async () => {
    const outcome = await submit({
      intent: "manual_deploy",
      message: "Undeliverable deploy",
      review: await currentReview(),
      inngest: failingInngest(),
    });

    expect(outcome).toEqual({
      state: "attempt_dispatch_failed",
      environmentDeploymentId: expect.any(String),
    });
    if (outcome.state !== "attempt_dispatch_failed") return;
    const [saved] = await savedRows();
    expect(saved).toEqual({ id: expect.any(String), actorId: userId, message: "Undeliverable deploy" });
    expect(await attemptRows()).toEqual([
      expect.objectContaining({
        id: outcome.environmentDeploymentId,
        status: "failed",
        failureCode: ENVIRONMENT_DEPLOYMENT_DISPATCH_FAILURE_CODE,
        savedStateSnapshotId: saved?.id,
      }),
    ]);

    await renameWorkingService("Edited after the failure");
    const sent: SentDispatch[] = [];
    const retry = await harness.runEffect(
      createRetryAttempt({
        environmentId,
        userId,
        failedDeploymentId: outcome.environmentDeploymentId,
      }).pipe(Effect.provideService(InngestClient, recordingInngest(sent))),
    );

    expect(await savedRows()).toHaveLength(1);
    expect(await attemptRows()).toEqual([
      expect.objectContaining({ id: outcome.environmentDeploymentId, status: "failed" }),
      {
        id: retry.data.environmentDeploymentId,
        status: "queued",
        failureCode: null,
        savedStateSnapshotId: saved?.id,
        retryOfDeploymentId: outcome.environmentDeploymentId,
        triggerOrigin: { origin: "manual", actorId: userId },
      },
    ]);
    expect(sent).toEqual([
      { environmentDeploymentId: retry.data.environmentDeploymentId, environmentId },
    ]);
  });

  it("refuses an ordinary retry once the failed attempt carries a Volume removal", async () => {
    const outcome = await submit({
      intent: "manual_deploy",
      message: "Undeliverable volume removal",
      review: await currentReview(),
      inngest: failingInngest(),
    });
    if (outcome.state !== "attempt_dispatch_failed") {
      throw new Error(`expected a dispatch failure, got ${outcome.state}`);
    }
    await harness.runEffect(
      stageVolumeRemoveAttempt({
        organizationId,
        requestedByUserId: userId,
        environmentId,
        environmentDeploymentId: outcome.environmentDeploymentId,
        environmentResourceId: volumeId,
        volumes: [{ machine_id: machineId, name: `vol-${volumeId}` }],
      }),
    );

    await expect(
      harness.runEffect(
        createRetryAttempt({
          environmentId,
          userId,
          failedDeploymentId: outcome.environmentDeploymentId,
        }).pipe(Effect.provideService(InngestClient, recordingInngest([]))),
      ),
    ).rejects.toMatchObject({
      _tag: "Validation",
      message: "Volume removal retries require a fresh destructive review from the environment canvas.",
    });
    expect(await attemptRows()).toHaveLength(1);
  });
});
