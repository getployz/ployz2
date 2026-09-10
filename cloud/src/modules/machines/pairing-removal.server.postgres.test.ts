import { loadOrganizationConnections } from "#/modules/machines/connections.server";
import type { Client, ConnectOptions, TailcatRemoval } from "@ployz/sdk";
import { Effect, Exit, Layer, ManagedRuntime, Scope } from "effect";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { asTestDouble } from "#/lib/test-double";
import { startGithubPostgresTestHarness, type GithubPostgresTestHarness } from "#/modules/github/github-ingestion.postgres-test-harness";
import { hashEnrollmentToken } from "#/modules/machines/enrollment.server";
import { requestMachineRemoveAttempt, claimMachineRemoveAttempt, completeMachineRemoveAttempt } from "#/modules/machines/machine-removal.repository";
import { disableOrganizationPairing, loadTeardownConnections, revokeOrganizationPairing } from "#/modules/machines/pairing-removal.server";
import { OrganizationRuntime, OrganizationRuntimeLive } from "#/modules/runtime/organization-runtime.server";
import { makePloyzLayer } from "#/modules/runtime/ployz.server";
import { Database } from "#/server/database.server";
import { makeSecretEncryption, SecretEncryption } from "#/utils/encrypted-secret.server";

const organizationId = "00000000-0000-4000-8000-000000008811";
const machineId = "00000000000000000000000000008811";
const otherMachineId = "00000000000000000000000000008812";
const pairing = "ppair_fixture_removal_generation";
const capability = "tailcat://fixture-removal";
const encryption = makeSecretEncryption("fixture-removal-encryption-1234567890");

describe("protected pairing removal", () => {
  let harness: GithubPostgresTestHarness;
  beforeAll(async () => { harness = await startGithubPostgresTestHarness(); }, 60_000);
  afterAll(async () => { await harness.stop(); });
  beforeEach(async () => {
    await harness.pool.query("truncate organization cascade");
    await harness.pool.query("insert into organization (id,name,slug) values ($1,'Removal','removal')", [organizationId]);
    await seedPairing();
  });

  async function seedPairing(secret = pairing, expected = capability, preferred = true) {
    await harness.pool.query(`insert into organization_pairing
      (organization_id, encrypted_pairing_secret, founder_public_key, founder_claim_machine_id)
      values ($1,$2,'founder-key',$3)`, [organizationId, encryption.encrypt(secret), machineId]);
    await harness.pool.query(`insert into organization_machine
      (organization_id,machine_id,cluster_key,encrypted_tailcat,is_dial_entry)
      values ($1,$2,$3,$4,$5)`, [organizationId, machineId, hashEnrollmentToken(secret), encryption.encrypt(expected), preferred]);
  }

  function fixture() {
    const endpoint = { current: capability, paired: true, online: true, loseAck: false, failBeforeRotation: false };
    const mutations: TailcatRemoval[] = [];
    const dialed: ConnectOptions[] = [];
    let prepared = 0;
    let closed = 0;
    const ployz = makePloyzLayer({
      prepareTailcatRemoval: async (expected) => { prepared += 1; return `${expected}-successor`; },
      connect: async (options) => {
        dialed.push(options);
        if (!("connections" in options)) throw new Error("shared connection required");
        const [connection] = options.connections;
        if (!endpoint.online || !connection || !("tailcat" in connection) || connection.tailcat !== endpoint.current || connection.machine_id !== machineId) {
          throw new Error("Endpoint unavailable");
        }
        const client = asTestDouble<Client>()({
          inspect: async () => asTestDouble<Awaited<ReturnType<Client["inspect"]>>>()({ cloud_paired: endpoint.paired }),
          removeCloudPairing: async (removal: TailcatRemoval) => {
            mutations.push(removal);
            const saved = await harness.pool.query("select removal_endpoints from organization_pairing where organization_id=$1", [organizationId]);
            const retained = saved.rows[0].removal_endpoints[0];
            expect(encryption.decrypt(retained.encryptedSuccessor)).toBe(removal.successor);
            expect(removal.expected_pairing).toBe(pairing);
            endpoint.paired = false;
            if (endpoint.failBeforeRotation) throw new Error("Crashed after clearing pairing");
            endpoint.current = removal.successor;
            if (endpoint.loseAck) throw new Error("Rotation closed the old stream");
          },
          close: async () => { closed += 1; },
        });
        return client;
      },
    });
    const layer = Layer.mergeAll(ployz, Layer.succeed(Database, harness.database), Layer.succeed(SecretEncryption, encryption));
    const makeRuntime = () => ManagedRuntime.make(OrganizationRuntimeLive.pipe(Layer.provideMerge(layer)));
    return { endpoint, mutations, dialed, prepared: () => prepared, closed: () => closed, makeRuntime };
  }

  it("retires a successfully removed founder without retaining an impossible revocation endpoint", async () => {
    const userId = "00000000-0000-4000-8000-000000008813";
    await harness.pool.query('insert into "user" (id,email,name) values ($1,$2,$3)', [userId, "remove-founder@example.test", "Owner"]);
    await harness.pool.query("insert into enrollment_allocation (organization_id,cluster_key,assignments) values ($1,$2,$3)",
      [organizationId, hashEnrollmentToken(pairing), JSON.stringify([{ machine: { id: machineId } }])]);
    const attempt = await harness.runEffect(requestMachineRemoveAttempt({ organizationId, machineId, requestedByUserId: userId, confirmDataLoss: [] }));
    await harness.runEffect(claimMachineRemoveAttempt({ attemptId: attempt.id, inngestRunId: "remove-founder", now: new Date() }));
    await harness.runEffect(completeMachineRemoveAttempt({ attemptId: attempt.id, inngestRunId: "remove-founder", completion: { state: "succeeded" } }));
    expect((await harness.pool.query("select * from organization_machine")).rows).toEqual([]);
    const fake = fixture();
    fake.endpoint.online = false;
    const runtime = fake.makeRuntime();
    try {
      expect(await runtime.runPromise(revokeOrganizationPairing(organizationId))).toEqual({ confirmed: true, endpoints: [] });
      expect(fake.dialed).toEqual([]);
    } finally { await runtime.dispose(); }
  });

  it("disables and cancels local and remote sessions even when pairing decryption fails", async () => {
    const fake = fixture();
    const local = fake.makeRuntime();
    const remote = fake.makeRuntime();
    const scope = Effect.runSync(Scope.make());
    try {
      for (const runtime of [local, remote]) {
        expect(await runtime.runPromise(Effect.flatMap(OrganizationRuntime, (service) => service.open(organizationId)).pipe(
          Effect.provideService(Scope.Scope, scope),
        ))).toMatchObject({ status: "connected" });
      }
      const stale = makeSecretEncryption("stale-removal-encryption-1234567890");
      await harness.pool.query("update organization_pairing set encrypted_pairing_secret=$2 where organization_id=$1",
        [organizationId, stale.encrypt(pairing)]);
      await expect(local.runPromise(revokeOrganizationPairing(organizationId))).rejects.toMatchObject({ _tag: "Conflict" });
      expect((await harness.pool.query("select removal_started_at from organization_pairing")).rows[0].removal_started_at).toBeInstanceOf(Date);
      await expect.poll(fake.closed).toBe(2);
      expect(await local.runPromise(loadOrganizationConnections(organizationId))).toEqual({ kind: "missing" });
      expect(fake.mutations).toEqual([]);
    } finally {
      await Effect.runPromise(Scope.close(scope, Exit.void));
      await local.dispose();
      await remote.dispose();
    }
  });

  it.each([
    { status: "prepared", machineId, encryptedSuccessor: encryption.encrypt("successor-only") },
    { status: "confirmed", machineId, encryptedExpected: encryption.encrypt("retained-secret") },
    { status: "pending", machineId, encryptedExpected: encryption.encrypt(capability), encryptedSuccessor: encryption.encrypt("unexpected") },
    { status: "unknown", machineId, encryptedExpected: encryption.encrypt(capability) },
    { status: "pending", machineId: "invalid-id", encryptedExpected: encryption.encrypt(capability) },
    { status: "pending", machineId, encryptedExpected: { version: 2, iv: "", tag: "", ciphertext: "" } },
  ])("rejects malformed persisted removal state before dialing: %j", async (endpoint) => {
    await harness.pool.query("delete from organization_machine where organization_id=$1", [organizationId]);
    await harness.pool.query("update organization_pairing set removal_started_at=now(), removal_endpoints=$2 where organization_id=$1",
      [organizationId, JSON.stringify([endpoint])]);
    const fake = fixture();
    const runtime = fake.makeRuntime();
    try {
      await expect(runtime.runPromise(revokeOrganizationPairing(organizationId)))
        .rejects.toMatchObject({ _tag: "PairingRemovalStateInvalid" });
      await expect(runtime.runPromise(loadTeardownConnections(organizationId)))
        .rejects.toMatchObject({ _tag: "PairingRemovalStateInvalid" });
      expect(fake.dialed).toEqual([]);
      expect(fake.prepared()).toBe(0);
      expect(fake.mutations).toEqual([]);
      const saved = await harness.pool.query("select removal_endpoints from organization_pairing where organization_id=$1", [organizationId]);
      expect(saved.rows[0].removal_endpoints).toEqual([endpoint]);
    } finally { await runtime.dispose(); }
  });

  it("disables ordinary access, moves encrypted credentials, and retains the founding claim offline", async () => {
    await expect(harness.pool.query("update organization_pairing set removal_started_at=now() where organization_id=$1", [organizationId]))
      .rejects.toMatchObject({ constraint: "organization_pairing_removal_shape_check" });
    const fake = fixture();
    fake.endpoint.online = false;
    const runtime = fake.makeRuntime();
    try {
      const result = await runtime.runPromise(revokeOrganizationPairing(organizationId));
      expect(result).toEqual({ confirmed: false, endpoints: [{ machineId, status: "unconfirmed" }] });
      expect(await runtime.runPromise(loadOrganizationConnections(organizationId))).toEqual({ kind: "missing" });
      expect(fake.mutations).toHaveLength(0);
      const saved = await harness.pool.query("select * from organization_pairing where organization_id=$1", [organizationId]);
      expect(saved.rows[0].founder_claim_machine_id).toBe(machineId);
      expect(saved.rows[0].removal_started_at).toBeInstanceOf(Date);
      const retained = saved.rows[0].removal_endpoints[0];
      expect(encryption.decrypt(retained.encryptedExpected)).toBe(capability);
      expect(encryption.decrypt(retained.encryptedSuccessor)).toBe(`${capability}-successor`);
      expect(JSON.stringify(saved.rows)).not.toContain(capability);
      expect((await harness.pool.query("select * from organization_machine")).rows).toEqual([]);
      expect(await runtime.runPromise(loadTeardownConnections(organizationId))).toEqual([{ machine_id: machineId, tailcat: capability }]);
    } finally { await runtime.dispose(); }
  });

  it("confirms a lost removal acknowledgement through successor identity and erases credentials", async () => {
    const fake = fixture();
    fake.endpoint.loseAck = true;
    const runtime = fake.makeRuntime();
    try {
      expect(await runtime.runPromise(revokeOrganizationPairing(organizationId))).toEqual({
        confirmed: true, endpoints: [{ machineId, status: "confirmed" }],
      });
      expect(fake.mutations).toHaveLength(1);
      expect(fake.dialed.every((options) => "connections" in options && options.connections.length === 1)).toBe(true);
      expect((await harness.pool.query("select * from organization_pairing")).rows).toEqual([]);
      expect(await runtime.runPromise(loadOrganizationConnections(organizationId))).toEqual({ kind: "missing" });
      await seedPairing("ppair_new_generation", fake.endpoint.current);
      const next = await runtime.runPromise(loadOrganizationConnections(organizationId));
      expect(next).toMatchObject({ kind: "ready", connections: [{ tailcat: `${capability}-successor` }] });
      expect((await harness.pool.query("select is_dial_entry from organization_machine")).rows).toEqual([{ is_dial_entry: true }]);
    } finally { await runtime.dispose(); }
  });

  it("restarts Cloud with the same prepared successor and confirms without replaying the mutation", async () => {
    const fake = fixture();
    fake.endpoint.online = false;
    const first = fake.makeRuntime();
    await first.runPromise(revokeOrganizationPairing(organizationId));
    await first.dispose();
    fake.endpoint.online = true;
    fake.endpoint.current = `${capability}-successor`;
    fake.endpoint.paired = false;
    const restarted = fake.makeRuntime();
    try {
      expect((await restarted.runPromise(revokeOrganizationPairing(organizationId))).confirmed).toBe(true);
      expect(fake.prepared()).toBe(1);
      expect(fake.mutations).toHaveLength(0);
    } finally { await restarted.dispose(); }
  });

  it("does not treat cleared pairing or failed old dials as confirmation", async () => {
    const fake = fixture();
    fake.endpoint.failBeforeRotation = true;
    const runtime = fake.makeRuntime();
    try {
      expect((await runtime.runPromise(revokeOrganizationPairing(organizationId))).confirmed).toBe(false);
      expect(fake.endpoint.paired).toBe(false);
      expect(fake.endpoint.current).toBe(capability);
      fake.endpoint.failBeforeRotation = false;
      expect((await runtime.runPromise(revokeOrganizationPairing(organizationId))).confirmed).toBe(true);
      expect(fake.prepared()).toBe(1);
      expect(fake.mutations).toHaveLength(2);
    } finally { await runtime.dispose(); }
  });

  it("reports each retained endpoint and does not replay a mutation against another Machine", async () => {
    await harness.pool.query(`insert into organization_machine
      (organization_id,machine_id,cluster_key,encrypted_tailcat,is_dial_entry)
      values ($1,$2,$3,$4,false)`, [organizationId, otherMachineId, hashEnrollmentToken(pairing), encryption.encrypt("tailcat://offline-other")]);
    const fake = fixture();
    const runtime = fake.makeRuntime();
    try {
      const outcome = await runtime.runPromise(revokeOrganizationPairing(organizationId));
      expect(outcome.confirmed).toBe(false);
      expect(outcome.endpoints).toEqual(expect.arrayContaining([
        { machineId, status: "confirmed" }, { machineId: otherMachineId, status: "unconfirmed" },
      ]));
      expect(fake.mutations).toHaveLength(1);
      const saved = await harness.pool.query("select removal_endpoints from organization_pairing");
      expect(saved.rows[0].removal_endpoints.find((entry: { machineId: string }) => entry.machineId === machineId)).toEqual({
        machineId, status: "confirmed",
      });
    } finally { await runtime.dispose(); }
  });

  it("retains an unpublished founding claim as an explicit unconfirmed omission", async () => {
    await harness.pool.query("delete from organization_machine");
    const fake = fixture();
    const runtime = fake.makeRuntime();
    try {
      await runtime.runPromise(disableOrganizationPairing(organizationId));
      expect(await runtime.runPromise(revokeOrganizationPairing(organizationId))).toEqual({
        confirmed: false, endpoints: [{ machineId, status: "unconfirmed" }],
      });
      expect(fake.prepared()).toBe(0);
      expect(fake.dialed).toEqual([]);
    } finally { await runtime.dispose(); }
  });
});
