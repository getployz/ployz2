import { describe, expect, it } from "vitest";
import {
  automaticBuildConcurrency,
  machineUpdateForPolicyChange,
  policyChangeObserved,
} from "#/modules/machines/server-policy";

const GB = 1_000_000_000;

describe("Server Policy", () => {
  it("shows the automatic build concurrency the Server's daemon enforces", () => {
    expect(automaticBuildConcurrency(true, 64 * GB)).toBe(1);
    expect(automaticBuildConcurrency(false, null)).toBe(1);
    expect(automaticBuildConcurrency(false, 2 * GB)).toBe(1);
    expect(automaticBuildConcurrency(false, 8 * GB + 1)).toBe(2);
    expect(automaticBuildConcurrency(false, 64 * GB)).toBe(4);
  });

  it("sends only the changed values to the Engine", () => {
    expect(machineUpdateForPolicyChange({ acceptsBuilds: false })).toEqual({
      accepts_builds: false,
    });
    expect(machineUpdateForPolicyChange({ buildConcurrency: "automatic" })).toEqual({
      build_concurrency: { action: "automatic" },
    });
    expect(machineUpdateForPolicyChange({ buildConcurrency: 4 })).toEqual({
      build_concurrency: { action: "set", value: 4 },
    });
  });

  it("settles a requested change only once observation shows all of it", () => {
    const observed = { acceptsBuilds: true, buildConcurrency: null };
    expect(policyChangeObserved(observed, { buildConcurrency: "automatic" })).toBe(true);
    expect(policyChangeObserved(observed, { buildConcurrency: 2 })).toBe(false);
    expect(
      policyChangeObserved(observed, { acceptsBuilds: true, buildConcurrency: 2 }),
    ).toBe(false);
    expect(
      policyChangeObserved(
        { acceptsBuilds: false, buildConcurrency: 2 },
        { acceptsBuilds: false, buildConcurrency: 2 },
      ),
    ).toBe(true);
  });
});
