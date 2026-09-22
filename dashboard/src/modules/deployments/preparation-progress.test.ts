import { expect, it } from "vitest";
import type { MachineId } from "@ployz/sdk";
import { preparationProgressCollector } from "./preparation-progress";

it("turns phases and BuildKit steps into one step tree with attributed output", () => {
  let tick = 0;
  const progress = preparationProgressCollector(() => new Date(1_000 + tick++));
  expect(progress.event({ Build: { Stage: "Upload" } })).toEqual({
    progress: { phase: "build", serviceId: null, machineId: null, machineName: null, message: "Uploading source" },
    steps: [{ key: "stage:Upload", name: "Uploading source", startedAt: new Date(1_000), completedAt: null, cached: false, error: null }], output: [],
  });
  const building = progress.event({ Build: { Stage: "Building" } });
  expect(building.steps.map((step) => [step.key, step.completedAt])).toEqual([["stage:Upload", new Date(1_001)], ["stage:Building", null]]);
  expect(progress.event({ Build: { Output: Array.from(Buffer.from("WARNING: plugin\n")) } })).toEqual({
    progress: null, steps: [], output: [{ step: "stage:Building", stderr: false, text: "WARNING: plugin\n" }],
  });
  expect(progress.event({ Build: { Step: { id: "sha256:a", name: "[sdk 1/2] RUN cargo build", started: "2026-09-22T21:09:06Z", completed: null, cached: false, error: null } } }).steps).toEqual([
    { key: "sha256:a", name: "[sdk 1/2] RUN cargo build", startedAt: new Date("2026-09-22T21:09:06Z"), completedAt: null, cached: false, error: null },
  ]);
  expect(progress.event({ Build: { StepOutput: { step: "sha256:a", stderr: true, text: "Compiling\n" } } }).output).toEqual([{ step: "sha256:a", stderr: true, text: "Compiling\n" }]);
  expect(progress.event({ phase: "truncated", dropped: 3 }).output).toEqual([{ step: "stage:Building", stderr: true, text: "… output dropped\n" }]);
  expect(progress.event("Transfer").steps.map((step) => step.key)).toEqual(["stage:Building", "transfer"]);
  expect(progress.event({ Delivered: { image: "web:1", machine_id: "m1" as MachineId } }).output).toEqual([{ step: "transfer", stderr: false, text: "Delivered web:1 to m1\n" }]);
  expect(progress.finish("builder lost")).toEqual([{ key: "transfer", name: "Delivering images", startedAt: new Date(1_004), completedAt: new Date(1_005), cached: false, error: "builder lost" }]);
  expect(progress.finish()).toEqual([]);
});

it("preserves UTF-8 split between unattributed output events", () => {
  const progress = preparationProgressCollector();
  const bytes = Buffer.from("error: 🐴\n");
  const first = progress.event({ Build: { Output: Array.from(bytes.subarray(0, 9)) } });
  const second = progress.event({ Build: { Output: Array.from(bytes.subarray(9)) } });
  expect([...first.output, ...second.output].map((row) => row.text).join("")).toBe("error: 🐴\n");
});
