import { Effect } from "effect";
import { vi } from "vitest";
import * as runner from "#/server/run.server";

const run = runner.makeEffectRunner({ runPromiseExit: Effect.runPromiseExit });
// SAFETY: Unit boundary suites replace capabilities with effects requiring no services; PostgreSQL suites provide application services.
vi.spyOn(runner, "runAppEffect").mockImplementation(run as typeof runner.runAppEffect);
// SAFETY: The same service-free unit effects pass through the real Inngest error encoder.
vi.spyOn(runner, "runInngestEffect").mockImplementation(
  runner.makeInngestEffectRunner(run) as typeof runner.runInngestEffect,
);
