import { randomUUID } from "node:crypto";
import { chmod, readdir, readFile, writeFile } from "node:fs/promises";
import { setTimeout as delay } from "node:timers/promises";
import { resolve } from "node:path";
import type { Connection } from "@ployz/sdk";
import { eq, sql } from "drizzle-orm";
import { environmentDeployment } from "#/modules/deployments/tables";
import { Cause, Effect, Exit } from "effect";
import { loadOrganizationConnections } from "#/modules/machines/connections.server";
import { mintMachineEnrollment } from "#/modules/machines/enrollment.server";
import { revokeOrganizationPairing } from "#/modules/machines/pairing-removal.server";
import { member, user } from "#/modules/identity/tables";
import { organization } from "#/modules/organization/tables";
import { OrganizationRuntime } from "#/modules/runtime/organization-runtime.server";
import { Database } from "#/server/database.server";
import { AppRuntime } from "#/server/runtime.server";
import { runAppEffect } from "#/server/run.server";

type State = {
  readonly version: 1;
  readonly organizationId: string;
  readonly organizationSlug: string;
  readonly userId: string;
  readonly founderToken: string;
  readonly joinToken: string;
  readonly phase: "seeded" | "enrolled" | "runtime-gates" | "timing" | "deadline" | "fallback" | "offline-revoked" | "online-revoked";
  readonly selectedMachineId?: string;
};

function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

const statePath = (() => {
  const value = process.env["PLOYZ_QUALIFICATION_STATE"];
  if (!value) throw new Error("PLOYZ_QUALIFICATION_STATE is required");
  return resolve(value);
})();

function tokenFrom(command: string) {
  const token = /'(?<token>pmet_[^']+)'/.exec(command)?.groups?.["token"];
  if (!token) throw new Error("minted enrollment command did not contain a token");
  return token;
}

async function writeState(state: State) {
  await writeFile(statePath, `${JSON.stringify(state)}\n`, { mode: 0o600 });
  await chmod(statePath, 0o600);
}

async function readState() {
  const state = JSON.parse(await readFile(statePath, "utf8")) as State;
  assert(state.version === 1, "qualification state version");
  return state;
}

async function probe() {
  await runAppEffect(Effect.gen(function* () {
    const { drizzle } = yield* Database;
    yield* drizzle.execute<{ one: number }>(sql`select 1 as one`);
  }));
}

async function seed() {
  const organizationId = randomUUID();
  const userId = randomUUID();
  const organizationSlug = `qualify-${organizationId.slice(0, 12)}`;
  const operation = Effect.gen(function* () {
    const { drizzle } = yield* Database;
    yield* drizzle.insert(user).values({
      id: userId,
      email: `${organizationSlug}@qualification.invalid`,
      name: "Tailcat qualification",
    });
    yield* drizzle.insert(organization).values({
      id: organizationId,
      name: "Tailcat qualification",
      slug: organizationSlug,
    });
    yield* drizzle.insert(member).values({
      id: randomUUID(), userId, organizationId, role: "owner",
    });
    const deployments = yield* drizzle.select({ count: sql<number>`count(*)::int` })
      .from(environmentDeployment).where(eq(environmentDeployment.organizationId, organizationId));
    assert(Number(deployments[0]?.count ?? -1) === 0, "fresh organization queued deployment");
    const founder = yield* mintMachineEnrollment({ userId }, { organizationSlug });
    const join = yield* mintMachineEnrollment({ userId }, { organizationSlug });
    return {
      version: 1 as const,
      organizationId,
      organizationSlug,
      userId,
      founderToken: tokenFrom(founder.command),
      joinToken: tokenFrom(join.command),
      phase: "seeded" as const,
    };
  });
  const state = await runAppEffect(operation).catch((cause) => {
    throw new Error(Cause.pretty(cause as Cause.Cause<unknown>));
  });
  await writeState(state);
}

async function helperPids() {
  const pids = await readdir("/proc");
  const helpers: string[] = [];
  for (const pid of pids) {
    if (!/^\d+$/u.test(pid)) continue;
    try {
      const [status, command] = await Promise.all([
        readFile(`/proc/${pid}/status`, "utf8"),
        readFile(`/proc/${pid}/cmdline`, "utf8"),
      ]);
      if (new RegExp(`^PPid:\\s+${process.pid}$`, "m").test(status) && command.includes("ployz-tailcat")) {
        helpers.push(pid);
      }
    } catch (cause) {
      if ((cause as NodeJS.ErrnoException).code !== "ENOENT") throw cause;
    }
  }
  return helpers;
}

async function helpersReaped() {
  const deadline = Date.now() + 5_000;
  let helpers = await helperPids();
  while (helpers.length !== 0 && Date.now() < deadline) {
    await delay(25);
    helpers = await helperPids();
  }
  assert(helpers.length === 0, `Tailcat helpers survived scope disposal: ${helpers.length}`);
}

type RuntimeGateReceipt = {
  readonly phase: "runtime-gates" | "timing" | "deadline";
  readonly coldOpenMs?: number;
  readonly warmReadMs?: number;
  readonly deadlineMs?: number;
  readonly deadlineResult?: "frame" | "deadline";
  readonly streamedFrame?: boolean;
  readonly observedMachines?: number;
};

async function recordRuntimeGate(receipt: RuntimeGateReceipt, phase: Extract<State["phase"], RuntimeGateReceipt["phase"]>) {
  await helpersReaped();
  console.log(JSON.stringify(receipt));
  const state = await readState();
  await writeState({ ...state, phase });
}

async function runtimeGates() {
  const state = await readState();
  const receipt = await runAppEffect(Effect.scoped(Effect.gen(function* () {
    const opened = yield* (yield* OrganizationRuntime).open(state.organizationId);
    assert(opened.status === "connected", "OrganizationRuntime did not connect");
    if (opened.status !== "connected") return { phase: "runtime-gates" as const };
    const [details, enrollment, frame] = yield* Effect.all([
      opened.connected.inspect(),
      opened.connected.observeEnrollment(),
      opened.connected.watchFirstFrame(10_000),
    ], { concurrency: "unbounded" });
    assert(details.cloud_paired === true, "Cloud pairing missing");
    assert(enrollment.machines.length >= 2, "enrollment read omitted joined machine");
    assert(Array.isArray(frame.machines), "runtime watch frame missing machines");
    return {
      phase: "runtime-gates" as const,
      streamedFrame: true,
      observedMachines: enrollment.machines.length,
    };
  })));
  await recordRuntimeGate(receipt, "runtime-gates");
}

async function timing() {
  const state = await readState();
  const receipt = await runAppEffect(Effect.scoped(Effect.gen(function* () {
    const runtime = yield* OrganizationRuntime;
    const coldStarted = performance.now();
    const opened = yield* runtime.open(state.organizationId);
    const coldOpenMs = Math.round(performance.now() - coldStarted);
    assert(opened.status === "connected", "cold OrganizationRuntime open did not connect");
    if (opened.status !== "connected") return { phase: "timing" as const, coldOpenMs };
    const warmStarted = performance.now();
    const details = yield* opened.connected.inspect();
    const warmReadMs = Math.round(performance.now() - warmStarted);
    assert(details.cloud_paired === true, "warm shared RPC read lost pairing");
    return { phase: "timing" as const, coldOpenMs, warmReadMs };
  })));
  await recordRuntimeGate(receipt, "timing");
}

async function deadline() {
  const state = await readState();
  const receipt = await runAppEffect(Effect.scoped(Effect.gen(function* () {
    const opened = yield* (yield* OrganizationRuntime).open(state.organizationId);
    assert(opened.status === "connected", "deadline OrganizationRuntime open did not connect");
    if (opened.status !== "connected") return { phase: "deadline" as const, deadlineResult: "deadline" as const, deadlineMs: 0 };
    const watch = yield* opened.connected.watch();
    const iterator = watch[Symbol.asyncIterator]();
    const first = yield* Effect.tryPromise(() => iterator.next());
    assert(first.done === false, "runtime watch ended before the first frame");
    const started = performance.now();
    const result = yield* Effect.exit(
      Effect.tryPromise(() => iterator.next()).pipe(Effect.timeout(500)),
    );
    const deadlineMs = Math.round(performance.now() - started);
    assert(!Exit.isSuccess(result), "runtime watch produced an unexpected second frame before its deadline");
    assert(deadlineMs >= 450 && deadlineMs < 3_000, `runtime watch deadline exceeded bound: ${deadlineMs}ms`);
    return {
      phase: "deadline" as const,
      deadlineMs,
      deadlineResult: "deadline" as const,
    };
  })));
  await recordRuntimeGate(receipt, "deadline");
}

async function writeTailcatContext(connections: readonly Connection[]) {
  const destination = process.env["PLOYZ_QUALIFICATION_CONTEXT_OUT"];
  if (!destination) return;
  assert(connections.length >= 2, "Tailcat context omitted enrolled candidates");
  assert(connections.every((connection) => "tailcat" in connection && typeof connection.tailcat === "string"), "qualification context contains a non-Tailcat connection");
  await writeFile(destination, `${JSON.stringify({
    current_context: "qualify",
    contexts: { qualify: { connections } },
  })}\n`, { mode: 0o600 });
  await chmod(destination, 0o600);
}

async function runtime(expectedPhase: State["phase"]) {
  const state = await readState();
  const selectedMachineId = await runAppEffect(Effect.scoped(Effect.gen(function* () {
    const connections = yield* loadOrganizationConnections(state.organizationId);
    assert(connections.kind === "ready", "Cloud connections unavailable");
    if (connections.kind !== "ready") return undefined;
    assert(connections.connections.length >= 2, "Cloud did not retain both enrollment candidates");
    yield* Effect.promise(() => writeTailcatContext(connections.connections));
    const opened = yield* (yield* OrganizationRuntime).open(state.organizationId);
    assert(opened.status === "connected", "OrganizationRuntime did not connect");
    if (opened.status !== "connected") return undefined;
    const details = yield* opened.connected.inspect();
    assert(details.cloud_paired === true, "Cloud pairing missing");
    if (expectedPhase === "fallback") {
      assert(state.selectedMachineId !== undefined, "fallback has no recorded preferred Machine");
      assert(details.id !== state.selectedMachineId, "fallback retained the stopped preferred Machine");
    }
    return details.id;
  })));
  assert(selectedMachineId !== undefined, "runtime did not select a Machine");
  await writeState({ ...state, phase: expectedPhase, selectedMachineId });
  console.log(JSON.stringify({ phase: expectedPhase, selectedMachineId }));
}

async function revoke(expectedConfirmed: boolean, phase: Extract<State["phase"], "offline-revoked" | "online-revoked">) {
  const state = await readState();
  const outcome = await runAppEffect(revokeOrganizationPairing(state.organizationId));
  assert(outcome.confirmed === expectedConfirmed, "unexpected revocation confirmation");
  await writeState({ ...state, phase });
}

export async function runQualificationPhase() {
  try {
    switch (process.env["PLOYZ_QUALIFICATION_PHASE"]) {
      case "probe":
        await probe();
        break;
      case "seed":
        await seed();
        break;
      case "runtime":
        await runtime("enrolled");
        break;
      case "runtime-gates":
        await runtimeGates();
        break;
      case "timing":
        await timing();
        break;
      case "deadline":
        await deadline();
        break;
      case "fallback":
        await runtime("fallback");
        break;
      case "revoke-offline":
        await revoke(false, "offline-revoked");
        break;
      case "revoke-online":
        await revoke(true, "online-revoked");
        break;
      default:
        throw new Error("PLOYZ_QUALIFICATION_PHASE must be probe, seed, runtime, runtime-gates, timing, deadline, fallback, revoke-offline, or revoke-online");
    }
  } finally {
    await AppRuntime.dispose();
  }
}
