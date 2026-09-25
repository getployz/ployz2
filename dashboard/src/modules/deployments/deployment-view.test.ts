import { describe, expect, it } from "vitest";
import type { ContainerId, DeployOperation, MachineId, OperationRow } from "@ployz/sdk";
import { resolvedServiceSpecFixture } from "#/modules/runtime/runtime-watch-frame.test-fixture";
import { canonicalJson } from "#/modules/environment-design/canonical-json";
import { deploymentProgressForEvent, deploymentStatusLabel, deploymentView, type DeploymentViewInput } from "./deployment-view";

function row(index: number, operation?: DeployOperation): OperationRow {
  const spec = resolvedServiceSpecFixture();
  spec.container.environment = { SECRET: "never-publish" };
  return { index, machine_id: `machine-${index}` as MachineId, machine_name: `server-${index}`, service_name: `svc-${index}`, display_name: `api-${index}`,
    operation: operation ?? { type: "run_container", machine_id: `machine-${index}` as MachineId, spec, skip_health_monitor: false }, status: { type: "pending" } };
}
/** The Engine serializes keys alphabetically, unlike the planned rows. */
const engineOrdered = <T,>(value: T): T => JSON.parse(canonicalJson(value)) as T;
const context = { serviceIdFor: (name: string | null) => name };
const deployment = (status: DeploymentViewInput["deployment"]["status"], extra: Partial<DeploymentViewInput["deployment"]> = {}): DeploymentViewInput["deployment"] =>
  ({ status, failureCode: null, failureMessage: null, deployPreview: null, ...extra });

describe("deployment view projection", () => {
  it("retains concurrent phases, identities and health deadlines without resolved secrets", () => {
    const rows: OperationRow[] = [
      { ...row(0), status: { type: "completed" } },
      { ...row(1), status: { type: "running", phase: { type: "waiting_for_health", container_id: "container-1" as ContainerId, health: "starting", elapsed_ms: 12000, deadline_ms: 60000 } } },
      { ...row(2), status: { type: "running", phase: { type: "creating_container" } } },
    ];
    const progress = deploymentProgressForEvent({ type: "progress", completed: 1, total: 3, rows }, rows, context);
    expect(JSON.stringify(progress)).not.toContain("never-publish");
    expect(JSON.stringify(progress)).not.toContain("environment");
    const view = deploymentView({ deployment: deployment("deploying"), progress, nodes: ["svc-0", "svc-1", "svc-2"].map((nodeId) => ({ nodeId, changed: true })) });
    expect(view.status).toBe("deploying");
    expect(view.nodes.map((n) => n.outcome)).toEqual(["deployed", "deploying", "deploying"]);
    expect(view.nodes[1]?.tail).toEqual(["server-1 · starting · 12s / 60s deadline"]);
    expect(view.nodes[2]?.tail).toEqual(["server-2 · Creating container"]);
  });

  it("matches the failed operation structurally in a mid-rollout failure, keeping its container ID", () => {
    const spec = resolvedServiceSpecFixture();
    const replacement = { machine_id: "machine-1" as MachineId, old_container_id: "old" as ContainerId, spec, skip_health_monitor: false };
    const rows = [row(0), row(1, { type: "replace_container", ...replacement }), row(2)] as const;
    const progress = deploymentProgressForEvent({ type: "outcome", outcome: engineOrdered({
      type: "failed", completed: [rows[0].operation], unexecuted: [rows[2].operation],
      failed: { type: "replacement_health", operation: replacement, error: { type: "health", container_id: "new-container" as ContainerId, failure: { type: "timed_out" } }, compensation: { type: "stop_first", stop_new_container: { type: "stopped" }, restart_old_container: { type: "failed", error: { type: "machine", action: "StartContainer", error: { code: "unavailable", message: "never-publish", details: { secret: "never-publish" } } } } } },
    } as const) }, rows, context);
    expect(progress.compensation).toEqual(["Replacement container stopped", "Previous container restart failed: StartContainer: unavailable"]);
    expect(JSON.stringify(progress)).not.toContain("never-publish");

    const view = deploymentView({ deployment: deployment("failed", { failureCode: "sdk_deploy_failed", deployPreview: {} }), progress, nodes: [
      { nodeId: "svc-0", changed: true }, { nodeId: "svc-1", changed: true }, { nodeId: "svc-2", changed: true }, { nodeId: "db", changed: false },
    ] });
    expect(view.nodes.map((n) => n.outcome)).toEqual(["deployed", "failed", "not_attempted", "unchanged"]);
    expect(view.nodes[1]).toMatchObject({ deploy: { state: "failed" }, failure: { message: "Health check timed out", containerId: "new-container" }, tail: ["server-1 · Health check timed out"] });
    expect(view.nodes[2]?.deploy.state).toBe("skipped");
    expect(view.nodes[3]).toMatchObject({ build: { state: "none" }, deploy: { state: "skipped" } });
    expect(deploymentStatusLabel(view)).toBe("Failed · 1 of 3 deployed");
  });

  it("fails built nodes on a build failure and leaves prebuilt ones not attempted", () => {
    const view = deploymentView({
      deployment: deployment("failed", { failureCode: "sdk_preparation_failed", failureMessage: "Dockerfile parse error" }),
      progress: { completed: 0, total: 0, outcome: null, rows: [], compensation: [], preparation: { phase: "build", serviceId: "web", machineId: "m", machineName: "builder", message: null } },
      nodes: [{ nodeId: "web", changed: true, built: true }, { nodeId: "worker", changed: true }],
    });
    expect(view.nodes[0]).toMatchObject({ outcome: "failed", build: { state: "failed" }, failure: { message: "Dockerfile parse error" }, tail: ["Dockerfile parse error"] });
    expect(view.nodes[1]).toMatchObject({ outcome: "not_attempted", build: { state: "none" }, deploy: { state: "skipped" } });
    expect(deploymentStatusLabel(view)).toBe("Failed · 0 of 2 deployed");
  });

  it("marks a node the attempt removed as Removed and counts it as deployed", () => {
    const remove = row(0, { type: "remove_container", machine_id: "machine-0" as MachineId, container_id: "gone" as ContainerId });
    const progress = deploymentProgressForEvent({ type: "outcome", outcome: engineOrdered({ type: "success", completed: [remove.operation] } as const) }, [remove], context);
    const view = deploymentView({ deployment: deployment("applied"), progress, nodes: [{ nodeId: "svc-0", changed: true, removed: true }, { nodeId: "db", changed: false }] });
    expect(view.nodes.map((n) => n.outcome)).toEqual(["removed", "unchanged"]);
    expect(deploymentStatusLabel(view)).toBe("Deployed");
  });

  it("does not claim an unknown runtime outcome was never attempted", () => {
    const view = deploymentView({ deployment: deployment("failed", { failureCode: "sdk_deploy_outcome_unknown", failureMessage: "Connection lost", deployPreview: {} }), progress: null, nodes: [{ nodeId: "web", changed: true }] });
    expect(view.nodes[0]).toMatchObject({ outcome: "failed", deploy: { state: "unknown" }, failure: { message: "Connection lost" } });
    expect(view.deploy.state).toBe("unknown");
  });
});
