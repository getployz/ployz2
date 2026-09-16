import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { asc, eq } from "drizzle-orm";
import { Effect } from "effect";
import { Inngest } from "inngest";
import * as schema from "#/db/schema";
import {
  type GithubPostgresTestHarness,
  startGithubPostgresTestHarness,
} from "#/modules/github/github-ingestion.postgres-test-harness";
import { InngestClient } from "#/modules/inngest/client";
import { publishEnvironmentSavedState } from "#/modules/environment-design/saved-state-operations.server";
import {
  admitEnvironmentDeployment,
  loadLatestSavedDeploymentTarget,
} from "./admission.server";
import {
  dispatchEnvironmentDeployment,
  ENVIRONMENT_DEPLOYMENT_DISPATCH_FAILURE_CODE,
} from "./dispatch.server";
import { commitFirstConnectAdmission } from "./first-connect.server";

const organizationId = "00000000-0000-4000-8000-000000000401";
const userId = "00000000-0000-4000-8000-000000000402";
const projectId = "00000000-0000-4000-8000-000000000403";
const environmentId = "00000000-0000-4000-8000-000000000404";
const firstVolumeId = "00000000-0000-4000-8000-000000000405";
const firstLineageId = "00000000-0000-4000-8000-000000000406";
const secondVolumeId = "00000000-0000-4000-8000-000000000407";
const secondLineageId = "00000000-0000-4000-8000-000000000408";
const machineId = "0123456789abcdef0123456789abcdef";
const encryptedPairingSecret = {
  version: 1 as const,
  iv: "iv",
  tag: "tag",
  ciphertext: "ciphertext",
};

function intent(volumes: "first" | "both") {
  return {
    version: 1 as const,
    environmentSlug: "production",
    services: [],
    variableGroups: [],
    volumes: [
      {
        resourceId: firstVolumeId,
        resourceLineageId: firstLineageId,
        name: volumes === "first" ? "First" : "First changed",
      },
      ...(volumes === "both"
        ? [
            {
              resourceId: secondVolumeId,
              resourceLineageId: secondLineageId,
              name: "Second",
            },
          ]
        : []),
    ],
  };
}

describe("Saved deployment admission", () => {
  let harness: GithubPostgresTestHarness;

  beforeAll(async () => {
    harness = await startGithubPostgresTestHarness();
  }, 60_000);

  afterAll(async () => {
    await harness?.stop();
  });

  beforeEach(async () => {
    await harness.pool.query(`
      truncate table environment_saved_state_snapshot, environment, project,
        "user", organization cascade;
      insert into organization (id, name, slug)
      values ('${organizationId}', 'Acceptance', 'admission');
      insert into "user" (id, email, name)
      values ('${userId}', 'owner@example.com', 'Owner');
      insert into project (id, organization_id, name, slug)
      values ('${projectId}', '${organizationId}', 'Cloud', 'cloud');
      insert into environment (
        id, project_id, organization_id, name, namespace, intent
      ) values (
        '${environmentId}', '${projectId}', '${organizationId}',
        'Production', 'production', '{"version":1,"environmentSlug":"production","services":[],"variableGroups":[],"volumes":[]}'
      );
    `);
  });

  async function publish(
    volumes: "first" | "both",
    previousId: string | null,
  ) {
    return harness.runTransaction(() =>
        publishEnvironmentSavedState(
          {
            environmentId,
            actorId: userId,
            message: null,
            basis: previousId
              ? { kind: "saved_revision", savedStateSnapshotId: previousId }
              : { kind: "no_saved_state" },
            intent: intent(volumes),
            destructiveVolumeReviews: [],
            revisionPolicy: "always_create",
          }
        ),
    );
  }

  function admit(savedStateSnapshotId: string) {
    return harness.runTransaction(() =>
        admitEnvironmentDeployment(
          {
            environmentId,
            savedStateSnapshotId,
            triggerOrigin: { origin: "manual", actorId: userId },
            message: null,
          },
        ),
    );
  }

  async function insertPairing() {
    await harness.db.insert(schema.organizationPairing).values({
      organizationId,
      encryptedPairingSecret,
      founderPublicKey: "founder-public-key",
      founderClaimMachineId: machineId,
    });
  }

  function connect() {
    return harness.runTransaction(() =>
        commitFirstConnectAdmission(
          {
            organizationId,
            machineId,
            encryptedPairingSecret,
          },
        ),
    );
  }

  it("serializes concurrent manual admissions into one queued target", async () => {
    const saved = await publish("first", null);

    const attempts = await Promise.all(
      Array.from({ length: 8 }, () =>
        admit(saved.savedStateSnapshotId).then(
          (value) => ({ ok: true as const, value }),
          (error) => ({ ok: false as const, error }),
        ),
      ),
    );

    expect(attempts.filter((attempt) => attempt.ok)).toHaveLength(1);
    expect(await harness.db.select().from(schema.environmentDeployment)).toHaveLength(
      1,
    );
  });

  it("refuses a second manual admit while one attempt is queued", async () => {
    const first = await publish("first", null);
    const queued = await admit(first.savedStateSnapshotId);
    const second = await publish("both", first.savedStateSnapshotId);

    await expect(admit(second.savedStateSnapshotId)).rejects.toMatchObject({
      _tag: "Conflict",
    });
    expect(await harness.db.select().from(schema.environmentDeployment)).toEqual([
      expect.objectContaining({
        id: queued.id,
        status: "queued",
        savedStateSnapshotId: first.savedStateSnapshotId,
      }),
    ]);
  });

  it("admits a later manual after the queued attempt starts", async () => {
    const first = await publish("first", null);
    const queued = await admit(first.savedStateSnapshotId);
    const second = await publish("both", first.savedStateSnapshotId);

    await harness.db
      .update(schema.environmentDeployment)
      .set({
        status: "planning",
        inngestRunId: "run-started",
        startedAt: new Date(),
      })
      .where(eq(schema.environmentDeployment.id, queued.id));
    const third = await publish("first", second.savedStateSnapshotId);
    const next = await admit(third.savedStateSnapshotId);

    expect(next.id).not.toBe(queued.id);
    const rows = await harness.db
      .select({
        id: schema.environmentDeployment.id,
        status: schema.environmentDeployment.status,
        savedStateSnapshotId: schema.environmentDeployment.savedStateSnapshotId,
      })
      .from(schema.environmentDeployment)
      .orderBy(asc(schema.environmentDeployment.createdAt));
    expect(rows).toEqual([
      {
        id: queued.id,
        status: "planning",
        savedStateSnapshotId: first.savedStateSnapshotId,
      },
      {
        id: next.id,
        status: "queued",
        savedStateSnapshotId: third.savedStateSnapshotId,
      },
    ]);
  });

  it("resolves automated admission to the exact latest Saved revision", async () => {
    const first = await publish("first", null);
    const latest = await publish("both", first.savedStateSnapshotId);

    const deployment = await harness.runTransaction(() =>
      Effect.gen(function* () {
        const target = yield* loadLatestSavedDeploymentTarget(environmentId);
        return yield* admitEnvironmentDeployment({
          environmentId,
          savedStateSnapshotId: target.savedStateSnapshotId,
          triggerOrigin: {
            origin: "github",
            deliveryId: "delivery-1",
            branchEvaluationRevision: 1,
            installationId: 17,
            repositoryId: 42,
          },
          message: null,
        });
      }),
    );
    const [row] = await harness.db
      .select({
        savedStateSnapshotId: schema.environmentDeployment.savedStateSnapshotId,
      })
      .from(schema.environmentDeployment)
      .where(eq(schema.environmentDeployment.id, deployment.id));

    expect(row?.savedStateSnapshotId).toBe(latest.savedStateSnapshotId);
  });

  it("re-dispatches a committed unowned row after a dispatch crash", async () => {
    const saved = await publish("first", null);
    const deployment = await admit(saved.savedStateSnapshotId);
    await harness.db
      .update(schema.environmentDeployment)
      .set({ dispatchRequestedAt: new Date() })
      .where(eq(schema.environmentDeployment.id, deployment.id));
    const enqueued: Array<{ environmentDeploymentId: string; environmentId: string }> = [];
    const inngest = new Inngest({ id: "admission-dispatch-test" });
    inngest.send = async (event) => {
      const payload = Array.isArray(event) ? event[0] : event;
      if (payload === undefined) return { ids: [] };
      enqueued.push(payload.data as (typeof enqueued)[number]);
      return { ids: [] };
    };

    await harness.runEffect(
      dispatchEnvironmentDeployment(
        { environmentDeploymentId: deployment.id, environmentId },
      ).pipe(Effect.provideService(InngestClient, inngest)),
    );
    await harness.runEffect(
      dispatchEnvironmentDeployment(
        { environmentDeploymentId: deployment.id, environmentId },
      ).pipe(Effect.provideService(InngestClient, inngest)),
    );

    expect(enqueued).toEqual([
      { environmentDeploymentId: deployment.id, environmentId },
      { environmentDeploymentId: deployment.id, environmentId },
    ]);
  });

  it("does not re-dispatch a run-owned row and terminalizes an enqueue failure", async () => {
    const saved = await publish("first", null);
    const deployment = await admit(saved.savedStateSnapshotId);
    const failing = new Inngest({ id: "admission-dispatch-fail-test" });
    failing.send = async () => {
      throw new Error("Inngest unavailable");
    };
    await expect(
      harness.runEffect(
        dispatchEnvironmentDeployment(
          { environmentDeploymentId: deployment.id, environmentId },
        ).pipe(Effect.provideService(InngestClient, failing)),
      ),
    ).rejects.toMatchObject({ _tag: "InngestEventSendError" });
    const [failed] = await harness.db
      .select({
        status: schema.environmentDeployment.status,
        failureCode: schema.environmentDeployment.failureCode,
      })
      .from(schema.environmentDeployment)
      .where(eq(schema.environmentDeployment.id, deployment.id));
    expect(failed).toEqual({
      status: "failed",
      failureCode: ENVIRONMENT_DEPLOYMENT_DISPATCH_FAILURE_CODE,
    });

    const second = await admit(saved.savedStateSnapshotId);
    await harness.db
      .update(schema.environmentDeployment)
      .set({ inngestRunId: "run-owned" })
      .where(eq(schema.environmentDeployment.id, second.id));
    const owned = new Inngest({ id: "admission-dispatch-owned-test" });
    owned.send = async () => ({ ids: [] });
    await expect(
      harness.runEffect(
        dispatchEnvironmentDeployment(
          { environmentDeploymentId: second.id, environmentId },
        ).pipe(Effect.provideService(InngestClient, owned)),
      ),
    ).rejects.toMatchObject({ _tag: "Conflict" });
  });

  it("fails linked awaiting volume removals when dispatch send fails", async () => {
    const saved = await publish("first", null);
    const deployment = await admit(saved.savedStateSnapshotId);
    await harness.db.insert(schema.volumeRemoveAttempt).values({
      organizationId,
      requestedByUserId: userId,
      environmentId,
      environmentDeploymentId: deployment.id,
      environmentResourceId: firstVolumeId,
      volumes: [{ machine_id: machineId, name: `vol-${firstVolumeId}` }],
      status: "awaiting_deployment",
    });
    const failing = new Inngest({ id: "admission-dispatch-volume-fail-test" });
    failing.send = async () => {
      throw new Error("Inngest unavailable");
    };
    await expect(
      harness.runEffect(
        dispatchEnvironmentDeployment(
          { environmentDeploymentId: deployment.id, environmentId },
        ).pipe(Effect.provideService(InngestClient, failing)),
      ),
    ).rejects.toMatchObject({ _tag: "InngestEventSendError" });
    const [failed] = await harness.db
      .select({
        status: schema.environmentDeployment.status,
        failureCode: schema.environmentDeployment.failureCode,
        savedStateSnapshotId: schema.environmentDeployment.savedStateSnapshotId,
      })
      .from(schema.environmentDeployment)
      .where(eq(schema.environmentDeployment.id, deployment.id));
    const [volume] = await harness.db.select().from(schema.volumeRemoveAttempt);
    expect(failed).toEqual({
      status: "failed",
      failureCode: ENVIRONMENT_DEPLOYMENT_DISPATCH_FAILURE_CODE,
      savedStateSnapshotId: saved.savedStateSnapshotId,
    });
    expect(volume).toMatchObject({
      environmentDeploymentId: deployment.id,
      status: "failed",
      inngestRunId: null,
    });
  });

  it("does not fail a worker-claimed attempt or its volumes on a delayed send error", async () => {
    const saved = await publish("first", null);
    const deployment = await admit(saved.savedStateSnapshotId);
    await harness.db.insert(schema.volumeRemoveAttempt).values({
      organizationId,
      requestedByUserId: userId,
      environmentId,
      environmentDeploymentId: deployment.id,
      environmentResourceId: firstVolumeId,
      volumes: [{ machine_id: machineId, name: `vol-${firstVolumeId}` }],
      status: "awaiting_deployment",
    });
    const racing = new Inngest({ id: "admission-dispatch-claimed-test" });
    racing.send = async () => {
      await harness.db
        .update(schema.environmentDeployment)
        .set({ inngestRunId: "run-claimed" })
        .where(eq(schema.environmentDeployment.id, deployment.id));
      throw new Error("send lost the race");
    };
    await expect(
      harness.runEffect(
        dispatchEnvironmentDeployment(
          { environmentDeploymentId: deployment.id, environmentId },
        ).pipe(Effect.provideService(InngestClient, racing)),
      ),
    ).rejects.toMatchObject({ _tag: "InngestEventSendError" });
    const [row] = await harness.db
      .select({
        status: schema.environmentDeployment.status,
        inngestRunId: schema.environmentDeployment.inngestRunId,
        failureCode: schema.environmentDeployment.failureCode,
      })
      .from(schema.environmentDeployment)
      .where(eq(schema.environmentDeployment.id, deployment.id));
    const [volume] = await harness.db.select().from(schema.volumeRemoveAttempt);
    expect(row).toEqual({
      status: "queued",
      inngestRunId: "run-claimed",
      failureCode: null,
    });
    expect(volume?.status).toBe("awaiting_deployment");
  });

  it("records a no-Saved first connection so a later callback cannot deploy", async () => {
    await insertPairing();

    expect(await connect()).toEqual([]);
    const saved = await publish("first", null);
    expect(saved.savedStateSnapshotId).toBeTypeOf("string");
    expect(await connect()).toEqual([]);

    const [pairing] = await harness.db
      .select({
        founderMachineId: schema.organizationPairing.founderMachineId,
        evaluatedAt:
          schema.organizationPairing.firstConnectDeploymentEvaluatedAt,
      })
      .from(schema.organizationPairing);
    expect(pairing).toMatchObject({
      founderMachineId: machineId,
      evaluatedAt: expect.any(Date),
    });
    expect(await harness.db.select().from(schema.environmentDeployment)).toEqual(
      [],
    );
  });

  it("returns the same unowned first-connect row for post-commit replay", async () => {
    await insertPairing();
    const saved = await publish("first", null);

    const first = await connect();
    const replay = await connect();

    expect(first).toHaveLength(1);
    expect(replay).toEqual(first);
    const [deployment] = await harness.db
      .select({
        id: schema.environmentDeployment.id,
        savedStateSnapshotId: schema.environmentDeployment.savedStateSnapshotId,
        triggerOrigin: schema.environmentDeployment.triggerOrigin,
      })
      .from(schema.environmentDeployment);
    expect(deployment).toEqual({
      id: first[0]?.environmentDeploymentId,
      savedStateSnapshotId: saved.savedStateSnapshotId,
      triggerOrigin: { origin: "first_connect", machineId },
    });
  });

  it("does not create or return a duplicate after Inngest owns the first attempt", async () => {
    await insertPairing();
    await publish("first", null);
    const [first] = await connect();
    if (first === undefined) throw new Error("missing first-connect deployment");
    await harness.db
      .update(schema.environmentDeployment)
      .set({
        status: "planning",
        inngestRunId: "run-first-connect",
        startedAt: new Date(),
      })
      .where(
        eq(
          schema.environmentDeployment.id,
          first.environmentDeploymentId,
        ),
      );

    expect(await connect()).toEqual([]);
    expect(await harness.db.select().from(schema.environmentDeployment)).toHaveLength(
      1,
    );
  });

  it("leaves a queued manual attempt unchanged when first-connect admits", async () => {
    await insertPairing();
    const first = await publish("first", null);
    const queued = await admit(first.savedStateSnapshotId);
    const later = await publish("both", first.savedStateSnapshotId);

    const connected = await connect();
    const [row] = await harness.db
      .select({
        id: schema.environmentDeployment.id,
        savedStateSnapshotId: schema.environmentDeployment.savedStateSnapshotId,
        triggerOrigin: schema.environmentDeployment.triggerOrigin,
      })
      .from(schema.environmentDeployment);

    expect(later.savedStateSnapshotId).not.toBe(first.savedStateSnapshotId);
    expect(connected).toEqual([
      {
        environmentDeploymentId: queued.id,
        environmentId,
      },
    ]);
    expect(row).toEqual({
      id: queued.id,
      savedStateSnapshotId: first.savedStateSnapshotId,
      triggerOrigin: { origin: "manual", actorId: userId },
    });
  });
});
