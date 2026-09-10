import "@tanstack/react-start/server-only";

import { createHash } from "node:crypto";
import type { Connection, MachineId } from "@ployz/sdk";
import { and, eq, sql } from "drizzle-orm";
import { Effect, Schema } from "effect";
import { rustMachineIdSchema } from "#/modules/machines/enrollment";
import {
  enrollmentAllocation,
  machineEnrollmentToken,
  organizationMachine,
} from "#/modules/machines/tables";
import { OrganizationRuntime } from "#/modules/runtime/organization-runtime.server";
import { Ployz } from "#/modules/runtime/ployz.server";
import { organizationPairing } from "#/modules/runtime/tables";
import { Database } from "#/server/database.server";
import { Conflict } from "#/server/public-error";
import { SecretEncryption } from "#/utils/encrypted-secret.server";

type Pairing = typeof organizationPairing.$inferSelect;
type RemovalEndpoint = NonNullable<Pairing["removalEndpoints"]>[number];
type RemovalAttempt = Pairing & {
  removalStartedAt: Date;
  removalEndpoints: RemovalEndpoint[];
};

const decrypt = Effect.fn("PairingRemoval.decrypt")(function* (
  value: Parameters<SecretEncryption["Service"]["decrypt"]>[0],
) {
  const encryption = yield* SecretEncryption;
  return yield* Effect.try({
    try: () => encryption.decrypt(value),
    catch: () => new Conflict({ message: "The protected removal credential could not be read." }),
  });
});

/** Move usable credentials out of ordinary lookup before doing any remote work. */
export const disableOrganizationPairing = Effect.fn("PairingRemoval.disable")(
  function* (organizationId: string) {
    const database = yield* Database;
    const disabled = yield* database.transaction(Effect.gen(function* () {
      const { drizzle } = yield* Database;
      const [pairing] = yield* drizzle.select().from(organizationPairing)
        .where(eq(organizationPairing.organizationId, organizationId)).for("update");
      if (!pairing) return null;
      const secret = yield* decrypt(pairing.encryptedPairingSecret);
      const generation = createHash("sha256").update(secret).digest("hex");
      let attempt: RemovalAttempt;
      if (pairing.removalStartedAt !== null && pairing.removalEndpoints !== null) {
        attempt = { ...pairing, removalStartedAt: pairing.removalStartedAt, removalEndpoints: pairing.removalEndpoints };
      } else {
        const candidates = yield* drizzle.select().from(organizationMachine)
          .where(eq(organizationMachine.organizationId, organizationId));
        const [allocation] = yield* drizzle.select().from(enrollmentAllocation).where(and(
          eq(enrollmentAllocation.organizationId, organizationId),
          eq(enrollmentAllocation.clusterKey, generation),
        ));
        const removalEndpoints: RemovalEndpoint[] = candidates.map((candidate) => ({
          machineId: candidate.machineId,
          encryptedExpected: candidate.encryptedTailcat,
          encryptedSuccessor: null,
          confirmed: false,
        }));
        // A claim or reserved Join may have reached a Machine before publication was acknowledged.
        const intendedMachines = [pairing.founderClaimMachineId, ...(allocation?.assignments.map((assignment) => assignment.machine.id) ?? [])];
        for (const machineId of intendedMachines) {
          if (!removalEndpoints.some((endpoint) => endpoint.machineId === machineId)) {
            removalEndpoints.push({ machineId, encryptedExpected: null, encryptedSuccessor: null, confirmed: false });
          }
        }
        const removalStartedAt = new Date();
        yield* drizzle.update(organizationPairing).set({ removalStartedAt, removalEndpoints })
          .where(eq(organizationPairing.organizationId, organizationId));
        // This also retires the old generation's preferred-entry flag.
        yield* drizzle.delete(organizationMachine).where(eq(organizationMachine.organizationId, organizationId));
        yield* drizzle.delete(machineEnrollmentToken).where(eq(machineEnrollmentToken.organizationId, organizationId));
        attempt = { ...pairing, removalStartedAt, removalEndpoints };
      }
      yield* drizzle.execute(sql`select pg_notify('ployz_pairing_removed', ${JSON.stringify({ organizationId, generation })})`);
      return { attempt, generation };
    }));
    if (disabled !== null) {
      const runtime = yield* OrganizationRuntime;
      yield* runtime.cancel(organizationId, disabled.generation);
    }
    return disabled?.attempt ?? null;
  },
);

const loadCurrentAttempt = Effect.fn("PairingRemoval.loadCurrent")(
  function* (attempt: RemovalAttempt) {
    const { drizzle } = yield* Database;
    const [current] = yield* drizzle.select().from(organizationPairing).where(and(
      eq(organizationPairing.organizationId, attempt.organizationId),
      eq(organizationPairing.removalStartedAt, attempt.removalStartedAt),
    )).for("update");
    if (current?.removalEndpoints === null || current?.removalEndpoints === undefined) {
      return yield* new Conflict({ message: "The pairing removal attempt is no longer current." });
    }
    return { ...current, removalEndpoints: current.removalEndpoints };
  },
);

const prepareEndpointRemoval = Effect.fn("PairingRemoval.prepareEndpoint")(
  function* (attempt: RemovalAttempt, machineId: string) {
    const database = yield* Database;
    const ployz = yield* Ployz;
    const encryption = yield* SecretEncryption;
    return yield* database.transaction(Effect.gen(function* () {
      const { drizzle } = yield* Database;
      const current = yield* loadCurrentAttempt(attempt);
      const endpoint = current.removalEndpoints.find((entry) => entry.machineId === machineId);
      if (!endpoint || endpoint.confirmed || endpoint.encryptedExpected === null) return null;
      const expected = yield* decrypt(endpoint.encryptedExpected);
      const preparedNow = endpoint.encryptedSuccessor === null;
      const successor = endpoint.encryptedSuccessor === null
        ? yield* ployz.prepareTailcatRemoval(expected)
        : yield* decrypt(endpoint.encryptedSuccessor);
      if (preparedNow) {
        yield* drizzle.update(organizationPairing).set({
          removalEndpoints: current.removalEndpoints.map((entry) => entry.machineId === machineId
            ? { ...entry, encryptedSuccessor: encryption.encrypt(successor) } : entry),
        }).where(eq(organizationPairing.organizationId, attempt.organizationId));
      }
      return { expected, successor, preparedNow };
    }));
  },
);

const confirmEndpointRemoval = Effect.fn("PairingRemoval.confirmEndpoint")(
  function* (machineId: MachineId, successor: string) {
    const ployz = yield* Ployz;
    return yield* Effect.scoped(Effect.gen(function* () {
      const session = yield* ployz.connect({ connections: [{ machine_id: machineId, tailcat: successor }], timeoutMs: 10_000 });
      return (yield* session.inspect()).cloud_paired === false;
    })).pipe(Effect.catch(() => Effect.succeed(false)));
  },
);

/** Endpoint failures are unconfirmed outcomes, never evidence that old access was revoked. */
export const revokeOrganizationPairing = Effect.fn("PairingRemoval.revoke")(
  function* (organizationId: string) {
    const attempt = yield* disableOrganizationPairing(organizationId);
    const database = yield* Database;
    if (attempt === null) {
      const candidates = yield* database.drizzle.select({ machineId: organizationMachine.machineId })
        .from(organizationMachine).where(eq(organizationMachine.organizationId, organizationId));
      return { confirmed: candidates.length === 0, endpoints: candidates.map(({ machineId }) => ({ machineId, status: "unconfirmed" as const })) };
    }
    const ployz = yield* Ployz;
    const expectedPairing = yield* decrypt(attempt.encryptedPairingSecret);
    yield* Effect.forEach(attempt.removalEndpoints, (endpoint) => Effect.gen(function* () {
      const machineId = yield* Schema.decodeUnknownEffect(rustMachineIdSchema)(endpoint.machineId);
      const removal = yield* prepareEndpointRemoval(attempt, endpoint.machineId);
      if (removal === null) return;
      let confirmed = !removal.preparedNow && (yield* confirmEndpointRemoval(machineId, removal.successor));
      if (!confirmed) {
        // One selected endpoint, one mutation dispatch. A lost response is not a fallback signal.
        yield* Effect.scoped(Effect.gen(function* () {
          const session = yield* ployz.connect({ connections: [{ machine_id: machineId, tailcat: removal.expected }], timeoutMs: 10_000 });
          yield* session.removeCloudPairing({ expected: removal.expected, successor: removal.successor, expected_pairing: expectedPairing });
        })).pipe(Effect.ignore);
        for (let probe = 0; probe < 3 && !confirmed; probe += 1) {
          if (probe > 0) yield* Effect.sleep(500);
          confirmed = yield* confirmEndpointRemoval(machineId, removal.successor);
        }
      }
      if (!confirmed) return;
      yield* database.transaction(Effect.gen(function* () {
        const { drizzle } = yield* Database;
        const current = yield* loadCurrentAttempt(attempt);
        yield* drizzle.update(organizationPairing).set({
          removalEndpoints: current.removalEndpoints.map((entry) => entry.machineId === endpoint.machineId
            ? { ...entry, encryptedExpected: null, encryptedSuccessor: null, confirmed: true } : entry),
        }).where(eq(organizationPairing.organizationId, organizationId));
      }));
    }).pipe(Effect.ignore), { concurrency: 4, discard: true });
    return yield* database.transaction(Effect.gen(function* () {
      const { drizzle } = yield* Database;
      const current = yield* loadCurrentAttempt(attempt);
      const confirmed = current.removalEndpoints.every((endpoint) => endpoint.confirmed);
      if (confirmed) yield* drizzle.delete(organizationPairing).where(eq(organizationPairing.organizationId, organizationId));
      return { confirmed, endpoints: current.removalEndpoints.map((endpoint) => ({
        machineId: endpoint.machineId,
        status: endpoint.confirmed ? "confirmed" as const : "unconfirmed" as const,
      })) };
    }));
  },
);

/** Only the explicitly admitted destructive teardown can use retained old credentials. */
export const loadTeardownConnections = Effect.fn("PairingRemoval.teardownConnections")(
  function* (organizationId: string) {
    const { drizzle } = yield* Database;
    const [pairing] = yield* drizzle.select().from(organizationPairing)
      .where(eq(organizationPairing.organizationId, organizationId));
    const connections: Connection[] = [];
    for (const endpoint of pairing?.removalEndpoints ?? []) {
      if (endpoint.encryptedExpected !== null) connections.push({
        machine_id: yield* Schema.decodeUnknownEffect(rustMachineIdSchema)(endpoint.machineId),
        tailcat: yield* decrypt(endpoint.encryptedExpected),
      });
    }
    return connections;
  },
);
