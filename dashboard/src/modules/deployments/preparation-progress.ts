import type { PreparationEvent } from "@ployz/sdk";
import type { PreparationProgress } from "./deployment-progress";

/** A build step as the engine reports it: a BuildKit vertex or a Ployz-owned phase. */
export type BuildStepWrite = { key: string; name: string; startedAt: Date | null; completedAt: Date | null; cached: boolean; error: string | null };
export type BuildOutputWrite = { step: string; stderr: boolean; text: string };
export type PreparationWrites = { progress: PreparationProgress | null; steps: BuildStepWrite[]; output: BuildOutputWrite[] };

const stageNames = {
  Admission: "Waiting for the builder", Queued: "Queued", Upload: "Uploading source", Preparation: "Preparing the builder",
  Building: "Building", Output: "Loading images", Cleanup: "Cleaning up",
} satisfies Record<string, string>;
const stageName = (stage: string) => Object.entries(stageNames).find(([key]) => key === stage)?.[1] ?? stage;

/**
 * Folds one attempt's preparation events into progress plus build-log writes.
 * Ployz phases become steps like BuildKit's own, so the log is one tree.
 * Provider errors and rejection dumps are never logs.
 */
export function preparationProgressCollector(now: () => Date = () => new Date()) {
  const decoder = new TextDecoder();
  let current: PreparationProgress = { phase: "selection", serviceId: null, machineId: null, machineName: null, message: null };
  let phase: { key: string; name: string; startedAt: Date } | null = null;
  const end = (error: string | null = null): BuildStepWrite[] => {
    if (!phase) return [];
    const step = { key: phase.key, name: phase.name, startedAt: phase.startedAt, completedAt: now(), cached: false, error };
    phase = null;
    return [step];
  };
  const begin = (key: string, name: string): BuildStepWrite[] => {
    const steps = end();
    phase = { key, name, startedAt: now() };
    return [...steps, { key, name, startedAt: phase.startedAt, completedAt: null, cached: false, error: null }];
  };
  const none = (): PreparationWrites => ({ progress: null, steps: [], output: [] });
  const line = (text: string, stderr = false): PreparationWrites => {
    if (!text) return none();
    const steps = phase ? [] : begin("stage:Building", stageNames.Building);
    return { progress: null, steps, output: [{ step: phase?.key ?? "stage:Building", stderr, text }] };
  };
  return {
    current: () => current,
    event(event: PreparationEvent): PreparationWrites {
      if (event === "Transfer") {
        current = { ...current, phase: "transfer", message: "Delivering images" };
        return { progress: current, steps: begin("transfer", "Delivering images"), output: [] };
      }
      if ("Selected" in event) {
        current = { ...current, machineId: event.Selected.machine.id, machineName: event.Selected.machine.name, message: "Builder selected" };
        return { progress: current, steps: [], output: [] };
      }
      if ("Delivered" in event) return line(`Delivered ${event.Delivered.image} to ${event.Delivered.machine_id}\n`);
      if ("phase" in event) return line("… output dropped\n", true);
      if ("Platforms" in event) return none();
      const build = event.Build;
      if ("Stage" in build) {
        const name = stageName(build.Stage);
        current = { ...current, phase: "build", message: name };
        return { progress: current, steps: begin(`stage:${build.Stage}`, name), output: [] };
      }
      if ("Output" in build) return line(decoder.decode(Uint8Array.from(build.Output), { stream: true }));
      if ("Step" in build) {
        const step = build.Step;
        return { progress: null, output: [], steps: [{ key: step.id, name: step.name, startedAt: step.started ? new Date(step.started) : null, completedAt: step.completed ? new Date(step.completed) : null, cached: step.cached, error: step.error }] };
      }
      if ("StepOutput" in build) return { progress: null, steps: [], output: [{ step: build.StepOutput.step, stderr: build.StepOutput.stderr, text: build.StepOutput.text }] };
      return none();
    },
    /** Close the open phase once preparation ends, recording its failure if any. */
    finish: (error: string | null = null) => end(error),
  };
}
