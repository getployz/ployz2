import type { Client, ConnectOptions, EnrollmentAssignment, EnrollmentSnapshot } from "@ployz/sdk";
import { registerRequestFromEnrollmentIdentity, rustMachineIdSchema } from "./enrollment";
import { ConfigProvider, Effect, Exit, Layer, Result, Schema } from "effect";
import { Inngest } from "inngest";
import {
  afterAll,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
} from "vitest";
import {
  type GithubPostgresTestHarness,
  startGithubPostgresTestHarness,
} from "#/modules/github/github-ingestion.postgres-test-harness";
import {
  completeMachineEnrollment,
  publishMachineEnrollment,
  loadOrganizationConnections,
  enrollMachine,
  registerThroughHeldList,
  reserveEnrollmentAssignment,
  EnrollmentRelay,
  hashEnrollmentToken,
  heldMachineIds,
  loadOrganizationEnrollmentStatus,
  mintMachineEnrollment,
  resetPendingOrganizationEnrollment,
  tryRevokeOrganizationRelayPairing,
  type EnrollmentRelayService,
} from "#/modules/machines/enrollment.server";
import { asTestDouble } from "#/lib/test-double";
import { OrganizationRuntime, OrganizationRuntimeLive } from "#/modules/runtime/organization-runtime.server";
import { makePloyzLayer, PloyzProviderError } from "#/modules/runtime/ployz.server";
import { InngestClient } from "#/modules/inngest/client";
import { AppConfig } from "#/server/config.server";
import { Database, DatabaseLive } from "#/server/database.server";
import {
  makeSecretEncryption,
  SecretEncryption,
} from "#/utils/encrypted-secret.server";

const organizationId = "00000000-0000-4000-8000-000000000401";
const userId = "00000000-0000-4000-8000-000000000402";
const tokens = ["pmet_founding_cas_a", "pmet_founding_cas_b"];
const heldMachineId = Schema.decodeUnknownSync(rustMachineIdSchema)("00000000000000000000000000000001");
const tailcat = "tailcat://protected-founder";
const [heldMachine] = heldMachineIds([{ machineId: heldMachineId }]);
if (heldMachine === undefined) throw new Error("Invalid test Machine id.");
const snapshot: EnrollmentSnapshot = { network: "10.42.0.0/16", machines: [], target_versions: {} };
const enrollmentSettings = {
  publicRelayUrl: "https://relay.example.test/",
  deploymentDialBearer: "pdial_test",
  encryption: makeSecretEncryption("test-app-encryption-secret-1234567890"),
};

function identity(index: number) {
  return {
    protocolVersion: 2 as const,
    machineId: Schema.decodeUnknownSync(rustMachineIdSchema)((index + 1).toString(16).padStart(32, "0")),
    initialPolicy: {
      labels: {},
      accepts_builds: true,
      accepts_services: true,
      accepts_ingress: true,
    },
    name: `node-${index}`,
    publicKey: Buffer.alloc(32, index + 1).toString("base64"),
    advertisedEndpoints: [`10.0.0.${index + 1}:51820`],
    publicIp: `203.0.113.${index + 1}`,
    requestedStorage: "none" as const,
  };
}

function fakeRelay(database: GithubPostgresTestHarness["database"]) {
  let listCalls = 0;
  let registerCalls = 0;
  const revokedPairings: string[] = [];
  const held: unknown[] = [];
  const fail = { list: false, revoke: false, publish: false, conflict: false };
  const published: EnrollmentAssignment[] = [];
  const inspectHolding: EnrollmentRelayService["inspectHolding"] = () =>
    Effect.gen(function* () {
      listCalls += 1;
      if (fail.list) {
        return yield* new PloyzProviderError({
          operation: "list held registers",
          cause: new Error("Relay unreachable"),
        });
      }
      if (held.length === 0) return { kind: "empty" as const };
      const machineIds = heldMachineIds(held);
      if (machineIds.length === 0) {
        return yield* new PloyzProviderError({
          operation: "list held registers",
          cause: new Error("Relay returned no usable Machine ids"),
        });
      }
      return { kind: "held" as const, held: machineIds };
    });
  const relay: EnrollmentRelayService = {
    inspectHolding,
    registerAvailable: (input) => registerThroughHeldList(input).pipe(
      Effect.provideService(Database, database),
      Effect.provide(makePloyzLayer({
        connect: async () => { throw new Error("unused"); },
        observeEnrollment: async () => snapshot,
        publishEnrollment: async (_url, _bearer, _pairing, _entry, assignment) => {
          registerCalls += 1;
          published.push(assignment);
          if (fail.conflict) throw Object.assign(new Error("Assignment conflicts"), { code: "conflict" });
          if (fail.publish) throw new Error("Lost publication response");
          return { assigned_machine: assignment.machine, visible_peers: [], target_versions: {} };
        },
      })),
    ),
    revokeIfEmpty: (input) =>
      Effect.gen(function* () {
        const holding = yield* inspectHolding(input);
        if (holding.kind === "held") return "held" as const;
        if (fail.revoke) {
          return yield* new PloyzProviderError({
            operation: "revoke relay pairing",
            cause: new Error("Relay revoke failed"),
          });
        }
        revokedPairings.push(input.pairing);
        return "revoked" as const;
      }),
    revokePairing: (input) =>
      Effect.gen(function* () {
        if (fail.revoke) {
          return yield* new PloyzProviderError({
            operation: "revoke relay pairing",
            cause: new Error("Relay revoke failed"),
          });
        }
        revokedPairings.push(input.pairing);
      }),
  };
  const connected: ConnectOptions[] = [];
  let closed = 0;
  const connection = { error: null as Error | null, beforeConnect: async () => {} };
  const coordinator = enrollmentTestClient(database, relay, async (options) => {
    connected.push(options);
    await connection.beforeConnect();
    if (connection.error) throw connection.error;
    return asTestDouble<Client>()({ close: async () => { closed += 1; } });
  });
  return {
    coordinator,
    connected,
    connection,
    closed: () => closed,
    published,
    held,
    fail,
    revokedPairings,
    listCalls: () => listCalls,
    registerCalls: () => registerCalls,
  };
}

function enrollmentTestClient(
  database: GithubPostgresTestHarness["database"],
  relay: EnrollmentRelayService,
  connect: (options: ConnectOptions) => Promise<Client>,
) {
  const provider = ConfigProvider.fromEnv({
    env: {
      NODE_ENV: "test",
      DATABASE_URL: "postgres://unused",
      ELECTRIC_URL: "http://localhost:30000",
      APP_URL: "https://cloud.example.test",
      BETTER_AUTH_SECRET: "better-auth-secret",
      GITHUB_CLIENT_ID: "github-client-id",
      GITHUB_CLIENT_SECRET: "github-client-secret",
      PLOYZ_RELAY_URL: enrollmentSettings.publicRelayUrl,
      PLOYZ_RELAY_DIAL_CREDENTIAL:
        enrollmentSettings.deploymentDialBearer ?? "",
      APP_ENCRYPTION_SECRET:
        "app-encryption-secret-at-least-32-characters",
    },
  });
  const config = AppConfig.layer.pipe(
    Layer.provide(ConfigProvider.layer(provider)),
  );
  const layer = Layer.mergeAll(
    config,
    makePloyzLayer({ connect }),
    Layer.succeed(Database, database),
    Layer.succeed(EnrollmentRelay, relay),
    Layer.succeed(InngestClient, new Inngest({ id: "enrollment-test" })),
    Layer.succeed(SecretEncryption, enrollmentSettings.encryption),
  );
  return {
    publish: (input: Parameters<typeof publishMachineEnrollment>[0]) => Effect.runPromise(
      Effect.result(publishMachineEnrollment(input).pipe(Effect.provide(layer))),
    ),
    connections: (id = organizationId) => Effect.runPromise(loadOrganizationConnections(id).pipe(Effect.provide(layer))),
    open: (id = organizationId) => Effect.runPromise(Effect.scoped(
      Effect.gen(function* () { return yield* (yield* OrganizationRuntime).open(id); }).pipe(
        Effect.provide(OrganizationRuntimeLive.pipe(Layer.provideMerge(layer))),
      ),
    )),
    enroll: (input: Parameters<typeof enrollMachine>[0]) =>
      Effect.runPromise(
        Effect.result(enrollMachine(input).pipe(Effect.provide(layer))),
      ),
    completeFounding: (input: Parameters<typeof completeMachineEnrollment>[0]) =>
      Effect.runPromise(
        Effect.result(
          completeMachineEnrollment(input).pipe(Effect.provide(layer)),
        ),
      ),
    resetPendingEnrollment: (_organizationId: string) =>
      Effect.runPromise(
        Effect.result(
          resetPendingOrganizationEnrollment(
            { userId },
            {
              organizationSlug: "enroll",
              confirmedFounderStoppedOrErased: true,
            },
          ).pipe(Effect.provide(layer)),
        ),
      ),
    tryRevokePairing: (organizationId: string) =>
      Effect.runPromise(
        tryRevokeOrganizationRelayPairing(organizationId).pipe(
          Effect.provide(layer),
        ),
      ),
  };
}

describe("organization enrollment coordinator", () => {
  let harness: GithubPostgresTestHarness;

  beforeAll(async () => {
    harness = await startGithubPostgresTestHarness();
  }, 60_000);

  afterAll(async () => {
    await harness.stop();
  });

  beforeEach(async () => {
    await harness.pool.query(`
      truncate table organization_machine, organization_pairing,
        machine_enrollment_token, "user", organization cascade;
      insert into organization (id, name, slug)
      values ('${organizationId}', 'Enroll', 'enroll');
      insert into "user" (id, email, name)
      values ('${userId}', 'enroll@example.com', 'Owner');
      insert into member (id, organization_id, user_id, role, created_at)
      values (gen_random_uuid(), '${organizationId}', '${userId}', 'owner', now());
      insert into machine_enrollment_token (
        organization_id, created_by_user_id, token_hash, expires_at
      ) values
        ('${organizationId}', '${userId}', '${hashEnrollmentToken(tokens[0] ?? "")}', now() + interval '1 day'),
        ('${organizationId}', '${userId}', '${hashEnrollmentToken(tokens[1] ?? "")}', now() + interval '1 day');
    `);
  });

  it("uses Actor and the managed database for enrollment commands", async () => {
    const provider = ConfigProvider.fromEnv({
      env: {
        NODE_ENV: "test",
        DATABASE_URL: harness.databaseUrl,
        ELECTRIC_URL: "http://localhost:30000",
        APP_URL: "https://cloud.example.test",
        BETTER_AUTH_SECRET: "better-auth-secret",
        GITHUB_CLIENT_ID: "github-client-id",
        GITHUB_CLIENT_SECRET: "github-client-secret",
        PLOYZ_RELAY_URL: "https://relay.example.test",
        APP_ENCRYPTION_SECRET:
          "app-encryption-secret-at-least-32-characters",
      },
    });
    const config = AppConfig.layer.pipe(
      Layer.provide(ConfigProvider.layer(provider)),
    );
    const layer = DatabaseLive.pipe(Layer.provideMerge(config));

    await Effect.runPromise(
      Effect.scoped(
        Effect.gen(function* () {
          const status = yield* loadOrganizationEnrollmentStatus(
            { userId },
            { organizationSlug: "enroll" },
          );
          expect(status).toBe("unclaimed");

          const minted = yield* mintMachineEnrollment(
            { userId },
            { organizationSlug: "enroll" },
          );
          expect(minted.command).toContain("ployz cloud enroll 'pmet_");

          const denied = yield* loadOrganizationEnrollmentStatus(
            { userId: "00000000-0000-4000-8000-000000000499" },
            { organizationSlug: "enroll" },
          ).pipe(Effect.exit);
          expect(Exit.isFailure(denied)).toBe(true);
        }).pipe(Effect.provide(layer)),
      ),
    );

    const tokens = await harness.pool.query<{ count: number }>(
      "select count(*)::int as count from machine_enrollment_token where created_by_user_id = $1",
      [userId],
    );
    expect(tokens.rows[0]?.count).toBe(3);
  });

  it("grants exactly one Organization founding attempt across twenty calls and multiple tokens", async () => {
    const fake = fakeRelay(harness.database);
    const coordinator = fake.coordinator;

    const outcomes = await Promise.all(
      Array.from({ length: 20 }, (_, index) =>
        coordinator.enroll({
          token: tokens[index % tokens.length] ?? "",
          identity: identity(index),
        }),
      ),
    );
    const directives = outcomes.flatMap((outcome) =>
      Result.isFailure(outcome) ? [] : [outcome.success],
    );

    expect(directives).toHaveLength(20);
    expect(directives.filter((value) => value.kind === "initialize")).toEqual([
      expect.objectContaining({ kind: "initialize", resumed: false }),
    ]);
    expect(directives.filter((value) => value.kind === "not_yet")).toHaveLength(
      19,
    );
    expect(fake.listCalls()).toBe(0);
  });

  it("resumes only the matching founder forever and parks every other Machine without Relay", async () => {
    const fake = fakeRelay(harness.database);
    fake.fail.list = true;
    const coordinator = fake.coordinator;

    const first = await coordinator.enroll({ token: tokens[0] ?? "", identity: identity(0) });
    expect(Result.isFailure(first)).toBe(false);
    if (Result.isFailure(first) || first.success.kind !== "initialize") return;

    await harness.pool.query(`
      update organization_pairing
      set updated_at = now() - interval '1 year'
      where organization_id = '${organizationId}'
    `);
    const resumed = await coordinator.enroll({ token: tokens[1] ?? "", identity: identity(0) });
    const waiter = await coordinator.enroll({ token: tokens[1] ?? "", identity: identity(1) });

    expect(Result.isFailure(resumed)).toBe(false);
    expect(Result.isFailure(waiter)).toBe(false);
    if (Result.isFailure(resumed) || Result.isFailure(waiter)) return;
    expect(resumed.success).toEqual({ ...first.success, resumed: true });
    expect(waiter.success).toEqual({ kind: "not_yet", retryAfter: 2 });
    expect(fake.listCalls()).toBe(0);
  });

  it("commits ready only after protected publication and scoped negotiation, then joins waiters", async () => {
    const fake = fakeRelay(harness.database);
    const coordinator = fake.coordinator;
    const first = await coordinator.enroll({ token: tokens[0] ?? "", identity: identity(0) });
    expect(Result.isFailure(first)).toBe(false);
    if (Result.isFailure(first) || first.success.kind !== "initialize") return;

    const beforeHeld = await coordinator.completeFounding({
      token: tokens[1] ?? "",
      machineId: heldMachineId,
      pairingCredential: first.success.pairing.secret,
    });
    expect(Result.isFailure(beforeHeld)).toBe(true);

    await coordinator.publish({ token: tokens[0] ?? "", machineId: identity(0).machineId, pairingCredential: first.success.pairing.secret, tailcat });
    fake.held.push({ machineId: heldMachineId });
    const completed = await coordinator.completeFounding({
      token: tokens[1] ?? "",
      machineId: heldMachineId,
      pairingCredential: first.success.pairing.secret,
    });
    const repeated = await coordinator.completeFounding({
      token: tokens[0] ?? "",
      machineId: heldMachineId,
      pairingCredential: first.success.pairing.secret,
    });
    expect(Result.isFailure(completed)).toBe(false);
    expect(Result.isFailure(repeated)).toBe(false);
    if (Result.isFailure(completed) || Result.isFailure(repeated)) return;
    expect(completed.success).toEqual({ machineId: heldMachineId });
    expect(repeated.success).toEqual(completed.success);

    const waiters = await Promise.all(
      Array.from({ length: 19 }, (_, index) =>
        coordinator.enroll({
          token: tokens[index % tokens.length] ?? "",
          identity: identity(index + 1),
        }),
      ),
    );
    expect(
      waiters.every(
        (outcome) => !Result.isFailure(outcome) && outcome.success.kind === "join",
      ),
    ).toBe(true);
    expect(fake.registerCalls()).toBe(19);
    expect(new Set(fake.published.map((a) => a.machine.subnet)).size).toBe(19);
    expect(fake.published.map((a) => a.machine.id).sort()).toEqual(
      Array.from({ length: 19 }, (_, index) => identity(index + 1).machineId).sort(),
    );

    const state = await harness.pool.query<{
      founder_machine_id: string | null;
    }>(
      `select founder_machine_id from organization_pairing
       where organization_id = '${organizationId}'`,
    );
    expect(state.rows[0]?.founder_machine_id).toBe(heldMachineId);

    fake.held.length = 0;
    const empty = await coordinator.enroll({
      token: tokens[0] ?? "",
      identity: identity(30),
    });
    expect(Result.isFailure(empty)).toBe(false);
    if (Result.isFailure(empty)) return;
    expect(empty.success).toEqual({ kind: "not_yet", retryAfter: 2 });

    fake.fail.list = true;
    const indeterminate = await coordinator.enroll({
      token: tokens[0] ?? "",
      identity: identity(31),
    });
    expect(Result.isFailure(indeterminate)).toBe(true);
    if (!Result.isFailure(indeterminate)) return;
    expect(indeterminate.failure._tag).toBe("PloyzProviderError");
  });

  it("retains the committed assignment after publication failure and conflicts on changed retry inputs", async () => {
    const fake = fakeRelay(harness.database);
    const founder = await fake.coordinator.enroll({ token: (tokens[0] ?? ""), identity: identity(0) });
    if (Result.isFailure(founder) || founder.success.kind !== "initialize") throw new Error("Founder missing");
    fake.held.push({ machineId: heldMachineId });
    await fake.coordinator.publish({ token: tokens[0] ?? "", machineId: identity(0).machineId, pairingCredential: founder.success.pairing.secret, tailcat });
    await fake.coordinator.completeFounding({ token: (tokens[0] ?? ""), machineId: heldMachineId, pairingCredential: founder.success.pairing.secret });
    fake.fail.publish = true;
    const failed = await fake.coordinator.enroll({ token: (tokens[1] ?? ""), identity: identity(1) });
    expect(failed).toMatchObject({ success: { kind: "not_yet" } });
    const saved = await harness.pool.query("select assignments from enrollment_allocation");
    expect(saved.rows[0].assignments).toEqual(fake.published);
    fake.fail.publish = false;
    // A fresh coordinator has no worker-local retry state.
    const fresh = fakeRelay(harness.database);
    fresh.held.push({ machineId: heldMachineId });
    const resumed = await fresh.coordinator.enroll({ token: (tokens[0] ?? ""), identity: identity(1) });
    expect(resumed).toMatchObject({ success: { kind: "join" } });
    expect(fresh.published).toEqual(fake.published);
    const changed = await fresh.coordinator.enroll({ token: (tokens[0] ?? ""), identity: { ...identity(1), requestedStorage: "zfs" } });
    expect(changed).toMatchObject({ failure: { _tag: "Conflict" } });
    expect(fresh.published).toHaveLength(1);
  });

  it("returns a permanent publication conflict without trying a stale Entry or freeing the assignment", async () => {
    const fake = fakeRelay(harness.database);
    const founder = await fake.coordinator.enroll({ token: tokens[0] ?? "", identity: identity(0) });
    if (Result.isFailure(founder) || founder.success.kind !== "initialize") throw new Error("Founder missing");
    fake.held.push({ machineId: heldMachineId });
    await fake.coordinator.publish({ token: tokens[0] ?? "", machineId: identity(0).machineId, pairingCredential: founder.success.pairing.secret, tailcat });
    await fake.coordinator.completeFounding({ token: tokens[0] ?? "", machineId: heldMachineId, pairingCredential: founder.success.pairing.secret });
    fake.held.push({ machineId: identity(0).machineId });
    fake.fail.conflict = true;

    const outcome = await fake.coordinator.enroll({ token: tokens[1] ?? "", identity: identity(1) });

    expect(outcome).toMatchObject({ failure: { _tag: "Conflict" } });
    expect(fake.registerCalls()).toBe(1);
    const saved = await harness.pool.query("select assignments from enrollment_allocation");
    expect(saved.rows[0].assignments).toEqual(fake.published);
    const retry = await fake.coordinator.enroll({ token: tokens[1] ?? "", identity: identity(1) });
    expect(retry).toMatchObject({ failure: { _tag: "Conflict" } });
    expect(fake.registerCalls()).toBe(2);
    expect(fake.published[1]).toEqual(fake.published[0]);
  });

  it("serializes identical requests and keeps organization and Cluster histories independent", async () => {
    const reserve = (organizationId: string, pairing: string, index: number) => harness.runEffect(
      reserveEnrollmentAssignment({ organizationId, pairing, identity: registerRequestFromEnrollmentIdentity(identity(index)), snapshot }),
    );
    const identical = await Promise.all(Array.from({ length: 20 }, () => reserve(organizationId, "cluster-a", 1)));
    expect(identical.every((assignment) => JSON.stringify(assignment) === JSON.stringify(identical[0]))).toBe(true);
    const next = await reserve(organizationId, "cluster-a", 2);
    expect(next.machine.subnet).not.toBe(identical[0]?.machine.subnet);
    const otherOrganization = "00000000-0000-4000-8000-000000000403";
    await harness.pool.query("insert into organization (id,name,slug) values ($1,'Other','other')", [otherOrganization]);
    const [otherCluster, otherOrg] = await Promise.all([
      reserve(organizationId, "cluster-b", 1), reserve(otherOrganization, "cluster-a", 1),
    ]);
    expect(otherCluster.machine.subnet).toBe(identical[0]?.machine.subnet);
    expect(otherOrg.machine.subnet).toBe(identical[0]?.machine.subnet);
    const histories = await harness.pool.query("select jsonb_array_length(assignments) as count from enrollment_allocation order by count");
    expect(histories.rows).toEqual([{ count: 1 }, { count: 1 }, { count: 2 }]);
  });

  it("releases transaction locks before publication and falls back to another held Entry", async () => {
    const tried: string[] = [];
    const assignment = await harness.runEffect(registerThroughHeldList({
      organizationId, relayUrl: "https://relay.example.test", bearer: "dial", pairing: "cluster-a",
      held: [heldMachine, identity(3).machineId], identity: registerRequestFromEnrollmentIdentity(identity(1)),
    }).pipe(Effect.provide(makePloyzLayer({
      connect: async () => { throw new Error("unused"); },
      observeEnrollment: async () => snapshot,
      publishEnrollment: async (_url, _bearer, pairing, entry, assignment) => {
        tried.push(entry);
        // Independent transaction must proceed while the network call is in flight.
        const next = await harness.runEffect(reserveEnrollmentAssignment({
          organizationId, pairing, identity: registerRequestFromEnrollmentIdentity(identity(2)), snapshot,
        }));
        expect(next.machine.subnet).not.toBe(assignment.machine.subnet);
        if (entry === heldMachine) throw new Error("Entry unreachable");
        return { assigned_machine: assignment.machine, visible_peers: [], target_versions: {} };
      },
    }))));
    expect(assignment.kind).toBe("registered");
    expect(tried).toEqual([heldMachine, identity(3).machineId]);
  });

  async function pendingFounder(fake: ReturnType<typeof fakeRelay>) {
    const result = await fake.coordinator.enroll({ token: tokens[0] ?? "", identity: identity(0) });
    if (Result.isFailure(result) || result.success.kind !== "initialize") throw new Error("Founder missing");
    return { token: tokens[0] ?? "", machineId: identity(0).machineId, pairingCredential: result.success.pairing.secret };
  }

  it("encrypts the authenticated candidate before completion and preserves it on exact retry", async () => {
    const fake = fakeRelay(harness.database);
    const attempt = await pendingFounder(fake);
    expect(await fake.coordinator.completeFounding(attempt)).toMatchObject({ failure: { _tag: "Conflict" } });
    expect(await fake.coordinator.completeFounding({ ...attempt, stage: "publish", tailcat })).toMatchObject({ success: { machineId: attempt.machineId } });
    const saved = await harness.pool.query("select encrypted_tailcat, cluster_key from organization_machine");
    expect(JSON.stringify(saved.rows)).not.toContain(tailcat);
    expect(enrollmentSettings.encryption.decrypt(saved.rows[0].encrypted_tailcat)).toBe(tailcat);
    expect(saved.rows[0].cluster_key).toBe(hashEnrollmentToken(attempt.pairingCredential));
    expect(await fake.coordinator.publish({ ...attempt, tailcat })).toMatchObject({ success: { machineId: attempt.machineId } });
    expect((await harness.pool.query("select encrypted_tailcat, cluster_key from organization_machine")).rows).toEqual(saved.rows);
    expect(await fake.coordinator.publish({ ...attempt, tailcat: "tailcat://replacement" })).toMatchObject({ failure: { _tag: "Conflict" } });
    expect((await harness.pool.query("select founder_machine_id, founder_claim_machine_id from organization_pairing")).rows).toEqual([
      { founder_machine_id: null, founder_claim_machine_id: attempt.machineId },
    ]);
    expect(fake.connected).toEqual([]);
    expect(fake.listCalls()).toBe(0);
    expect(await fake.coordinator.completeFounding(attempt)).toMatchObject({ success: { machineId: attempt.machineId } });
    expect(fake.connected).toEqual([expect.objectContaining({ connections: [{ tailcat, machine_id: attempt.machineId }] })]);
    expect(fake.closed()).toBe(1);
    expect(fake.listCalls()).toBe(0);
    expect(await fake.coordinator.enroll({ token: tokens[1] ?? "", identity: identity(0) })).toMatchObject({ success: { kind: "initialize", resumed: true } });
    expect(await fake.coordinator.tryRevokePairing(organizationId)).toBe(false);
    expect(await fake.coordinator.resetPendingEnrollment(organizationId)).toMatchObject({ failure: { _tag: "Conflict" } });
  });

  it("rejects unauthorized, stale, and wrong-Machine publications and completions before connecting", async () => {
    const fake = fakeRelay(harness.database);
    const attempt = await pendingFounder(fake);
    for (const input of [
      { ...attempt, token: "pmet_invalid" },
      { ...attempt, pairingCredential: "ppair_stale" },
      { ...attempt, machineId: identity(1).machineId },
    ]) {
      expect(Result.isFailure(await fake.coordinator.publish({ ...input, tailcat }))).toBe(true);
      expect(Result.isFailure(await fake.coordinator.completeFounding(input))).toBe(true);
    }
    expect((await harness.pool.query("select * from organization_machine")).rowCount).toBe(0);
    expect(fake.connected).toEqual([]);
    const changedMachine = await fake.coordinator.enroll({ token: attempt.token, identity: { ...identity(0), machineId: identity(1).machineId } });
    expect(changedMachine).toMatchObject({ success: { kind: "not_yet" } });
    await harness.pool.query("update machine_enrollment_token set expires_at = now() - interval '1 second'");
    expect(Result.isFailure(await fake.coordinator.publish({ ...attempt, tailcat }))).toBe(true);
    expect(Result.isFailure(await fake.coordinator.completeFounding(attempt))).toBe(true);
  });

  it("scopes colliding Machine IDs by Organization and current pairing", async () => {
    const fake = fakeRelay(harness.database);
    const attempt = await pendingFounder(fake);
    const otherId = "00000000-0000-4000-8000-000000000403";
    const otherToken = "pmet_other_organization";
    await harness.pool.query("insert into organization(id,name,slug) values($1,'Other','other')", [otherId]);
    await harness.pool.query("insert into machine_enrollment_token(organization_id,created_by_user_id,token_hash,expires_at) values($1,$2,$3,now()+interval '1 day')", [otherId, userId, hashEnrollmentToken(otherToken)]);
    const other = await fake.coordinator.enroll({ token: otherToken, identity: identity(0) });
    if (Result.isFailure(other) || other.success.kind !== "initialize") throw new Error("Other founder missing");
    const otherAttempt = { ...attempt, token: otherToken, pairingCredential: other.success.pairing.secret };
    expect(await fake.coordinator.publish({ ...attempt, token: otherToken, tailcat })).toMatchObject({ failure: { _tag: "Conflict" } });
    await fake.coordinator.publish({ ...attempt, tailcat });
    await fake.coordinator.publish({ ...otherAttempt, tailcat: "tailcat://other" });
    expect(await fake.coordinator.connections()).toEqual({ kind: "ready", connections: [{ tailcat, machine_id: attempt.machineId }] });
    expect(await fake.coordinator.connections(otherId)).toEqual({ kind: "ready", connections: [{ tailcat: "tailcat://other", machine_id: attempt.machineId }] });
    await harness.pool.query("update organization_pairing set encrypted_pairing_secret = $1::jsonb where organization_id = $2", [JSON.stringify(enrollmentSettings.encryption.encrypt("ppair_replacement")), organizationId]);
    expect(await fake.coordinator.connections()).toEqual({ kind: "ready", connections: [] });
    expect(await fake.coordinator.completeFounding(attempt)).toMatchObject({ failure: { _tag: "Conflict" } });
    expect(await fake.coordinator.publish({ ...attempt, tailcat })).toMatchObject({ failure: { _tag: "Conflict" } });
    expect(await fake.coordinator.completeFounding({ ...attempt, pairingCredential: "ppair_replacement" })).toMatchObject({ failure: { _tag: "Conflict" } });
    expect(fake.connected).toEqual([]);
  });

  it.each(["connection timed out", "Machine identity mismatch"])("keeps the published claim pending after %s", async (message) => {
    const fake = fakeRelay(harness.database);
    const attempt = await pendingFounder(fake);
    await fake.coordinator.publish({ ...attempt, tailcat });
    fake.connection.error = new Error(message);
    expect(await fake.coordinator.completeFounding(attempt)).toMatchObject({ failure: { _tag: "PloyzProviderError" } });
    expect((await harness.pool.query("select founder_machine_id from organization_pairing")).rows).toEqual([{ founder_machine_id: null }]);
    expect(await fake.coordinator.resetPendingEnrollment(organizationId)).toMatchObject({ failure: { _tag: "Conflict" } });
    expect(await fake.coordinator.enroll({ token: tokens[1] ?? "", identity: identity(0) })).toMatchObject({ success: { kind: "initialize", resumed: true } });
    expect(await fake.coordinator.enroll({ token: tokens[1] ?? "", identity: identity(1) })).toMatchObject({ success: { kind: "not_yet" } });
    fake.connection.error = null;
    expect(await fake.coordinator.completeFounding(attempt)).toMatchObject({ success: { machineId: attempt.machineId } });
    expect(fake.listCalls()).toBe(0);
    expect(fake.revokedPairings).toEqual([]);
  });

  it("accepts both exact completions racing the ready transaction", async () => {
    const fake = fakeRelay(harness.database);
    const attempt = await pendingFounder(fake);
    await fake.coordinator.publish({ ...attempt, tailcat });
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    fake.connection.beforeConnect = async () => {
      if (fake.connected.length === 2) release();
      await gate;
    };
    const results = await Promise.all([fake.coordinator.completeFounding(attempt), fake.coordinator.completeFounding(attempt)]);
    expect(results.every(Result.isSuccess)).toBe(true);
    expect(fake.closed()).toBe(2);
    expect((await harness.pool.query("select founder_machine_id from organization_pairing")).rows).toEqual([{ founder_machine_id: attempt.machineId }]);
  });

  it("rejects a pairing replacement that races shared connection confirmation", async () => {
    const fake = fakeRelay(harness.database);
    const attempt = await pendingFounder(fake);
    await fake.coordinator.publish({ ...attempt, tailcat });
    fake.connection.beforeConnect = async () => {
      await harness.pool.query("update organization_pairing set encrypted_pairing_secret = $1::jsonb where organization_id = $2", [
        JSON.stringify(enrollmentSettings.encryption.encrypt("ppair_replacement")), organizationId,
      ]);
    };
    expect(await fake.coordinator.completeFounding(attempt)).toMatchObject({ failure: { _tag: "Conflict" } });
    expect(fake.closed()).toBe(1);
    expect((await harness.pool.query("select founder_machine_id from organization_pairing")).rows).toEqual([{ founder_machine_id: null }]);
    await harness.pool.query("delete from organization_pairing where organization_id = $1", [organizationId]);
    expect(await fake.coordinator.connections()).toEqual({ kind: "missing" });
    expect(await fake.coordinator.tryRevokePairing(organizationId)).toBe(false);
  });

  it("never releases a claim on empty or failed transport evidence", async () => {
    const fake = fakeRelay(harness.database);
    expect(await fake.coordinator.resetPendingEnrollment(organizationId)).toMatchObject({ failure: { _tag: "Conflict" } });
    expect(await fake.coordinator.tryRevokePairing(organizationId)).toBe(true);
    const attempt = await pendingFounder(fake);
    for (const failed of [false, true]) {
      fake.fail.list = failed;
      expect(await fake.coordinator.resetPendingEnrollment(organizationId)).toMatchObject({ failure: { _tag: "Conflict" } });
    }
    expect(fake.listCalls()).toBe(0);
    expect(fake.revokedPairings).toEqual([]);
    expect((await harness.pool.query("select founder_claim_machine_id from organization_pairing")).rows).toEqual([{ founder_claim_machine_id: attempt.machineId }]);
    expect((await harness.pool.query("select * from machine_enrollment_token")).rowCount).toBe(2);
  });

  it("opens the actual Organization runtime from one protected candidate without Relay List", async () => {
    const fake = fakeRelay(harness.database);
    expect(await fake.coordinator.open()).toEqual({ status: "no_connection" });
    const attempt = await pendingFounder(fake);
    expect(await fake.coordinator.open()).toEqual({ status: "unreachable", error: null });
    await fake.coordinator.publish({ ...attempt, tailcat });
    expect(await fake.coordinator.open()).toMatchObject({ status: "connected" });
    expect(fake.connected).toEqual([expect.objectContaining({ connections: [{ tailcat, machine_id: attempt.machineId }] })]);
    expect(fake.closed()).toBe(1);
    fake.connection.error = new Error("candidate unreachable");
    expect(await fake.coordinator.open()).toMatchObject({ status: "unreachable", error: { _tag: "PloyzProviderError" } });
    expect(fake.listCalls()).toBe(0);
    expect((await harness.pool.query("select founder_machine_id from organization_pairing")).rows).toEqual([{ founder_machine_id: null }]);
  });

  it("keeps pairing decrypt failures in the typed Effect channel", async () => {
    const stale = makeSecretEncryption("stale-app-encryption-secret-1234567890");
    await harness.pool.query(
      `insert into organization_pairing (
        organization_id, encrypted_pairing_secret, founder_public_key
      ) values ($1, $2::jsonb, $3)`,
      [
        organizationId,
        JSON.stringify(stale.encrypt("ppair_stale")),
        identity(0).publicKey,
      ],
    );

    const fake = fakeRelay(harness.database);
    const enrolled = await fake.coordinator.enroll({
      token: tokens[0] ?? "",
      identity: identity(0),
    });
    expect(Result.isFailure(enrolled)).toBe(true);
    if (!Result.isFailure(enrolled)) return;
    expect(enrolled.failure._tag).toBe("PairingSecretDecryptFailure");

    const revoked = await fake.coordinator.tryRevokePairing(organizationId);
    expect(revoked).toBe(false);
    expect(fake.revokedPairings).toEqual([]);
  });

  it("rejects invalid pending and ready Organization Pairing shapes", async () => {
    await expect(
      harness.pool.query(`
        insert into organization_pairing (
          organization_id, encrypted_pairing_secret
        ) values ('${organizationId}', '{}'::jsonb)
      `),
    ).rejects.toMatchObject({ code: "23514" });

    await expect(
      harness.pool.query(`
        insert into organization_pairing (
          organization_id, encrypted_pairing_secret, founder_machine_id
        ) values (
          '${organizationId}', '{}'::jsonb,
          'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
        )
      `),
    ).resolves.toBeDefined();

    await harness.pool.query(
      `delete from organization_pairing where organization_id = '${organizationId}'`,
    );

    await expect(
      harness.pool.query(`
        insert into organization_pairing (
          organization_id, encrypted_pairing_secret,
          founder_public_key, founder_machine_id
        ) values (
          '${organizationId}', '{}'::jsonb, 'founder', 'not-a-machine'
        )
      `),
    ).rejects.toMatchObject({ code: "23514" });
  });
});
