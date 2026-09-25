import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { asTestDouble } from "#/lib/test-double";
import type { EnvironmentDeploymentSummary } from "#/modules/deployments/deployment-contract";
import type { DeploymentProgressRow } from "#/modules/deployments/deployment-progress";
import { deploymentView } from "#/modules/deployments/deployment-view";
import { DeploymentStatusCard } from "./deployment-status-card";

/** The Attempt Target here is the built services plus every service with rows; the read model supplies it in the app. */
function Card(props: Omit<Parameters<typeof DeploymentStatusCard>[0], "view">) {
  const { buildServiceIds } = props.deployment;
  const ids = new Set([...buildServiceIds, ...(props.progress?.rows ?? []).flatMap((r) => r.serviceId ? [r.serviceId] : [])]);
  const nodes = [...ids].map((nodeId) => ({ nodeId, changed: true, built: buildServiceIds.includes(nodeId) }));
  return <DeploymentStatusCard {...props} view={deploymentView({ deployment: props.deployment, progress: props.progress, nodes })} />;
}

const row = (serviceId: string, status: DeploymentProgressRow["status"]): DeploymentProgressRow => ({
  index: serviceId === "postgres" ? 0 : 1, serviceId, runtimeServiceId: serviceId, serviceName: serviceId,
  machineId: "machine", machineName: "server", displayName: null, target: null, updateOrder: "start_first",
  operation: "replace_container", status, phase: null, elapsedMs: null, deadlineMs: null, health: null,
  error: status === "failed" ? "Health check timed out" : null,
  containerId: status === "failed" ? "c0ffee" : null,
});
it("shows a deployed service independently of a later failure, and the failing container", () => {
  const deployment = asTestDouble<EnvironmentDeploymentSummary>()({ sourcePins: {}, buildServiceIds: [], status: "failed", createdAt: new Date(), serviceCount: 2, volumeRemoveAttempts: [], failureCode: "sdk_deploy_failed" });
  const progress = { completed: 1, total: 2, outcome: "failed" as const, rows: [row("postgres", "completed"), row("web", "failed")], compensation: [] };
  const render = (serviceId?: string) => renderToStaticMarkup(createElement(Card, {
    deployment, progress, serviceId, expanded: true, onExpandedChange() {}, showLogs: false, onLogsChange() {}, actions: null, logsPanel: null,
  }));
  const service = render("postgres");
  expect(service).toContain("Service deployed");
  expect(service).toContain("border-success/30");
  expect(service).not.toContain("Health check timed out");
  const environment = render();
  expect(environment).toContain("Failed · 1 of 2 deployed");
  expect(environment).not.toContain("Partial");
  expect(render("web")).toContain("Health check timed out");
  expect(render("web")).toContain("container c0ffee");
});

it("shows captured commit, selected Server, transfer phase and truncation", () => {
  const deployment = asTestDouble<EnvironmentDeploymentSummary>()({ sourcePins: { web: { commitSha: "a".repeat(40) } }, buildServiceIds: ["web"], status: "deploying", createdAt: new Date(), serviceCount: 1 });
  const progress = { completed: 0, total: 0, outcome: null, rows: [], compensation: [], preparation: { phase: "transfer" as const, serviceId: "web", machineId: "machine", machineName: "builder", message: null } };
  const html = renderToStaticMarkup(createElement(Card, { deployment, progress, expanded: true, onExpandedChange() {}, showLogs: false, onLogsChange() {}, actions: null, logsPanel: null }));
  expect(html).toContain("Transferring images");
  expect(html).toContain("a".repeat(40));
  expect(html).toContain("Build Server: builder");
  expect(html).not.toContain("Using prebuilt images");
});
it("keeps image-only deployments on the prebuilt path", () => {
  const deployment = asTestDouble<EnvironmentDeploymentSummary>()({ sourcePins: {}, buildServiceIds: [], status: "applied", createdAt: new Date(), serviceCount: 1 });
  const html = renderToStaticMarkup(createElement(Card, { deployment, progress: null, expanded: true, onExpandedChange() {}, showLogs: false, onLogsChange() {}, actions: null, logsPanel: null }));
  expect(html).toContain("Using prebuilt images");
  expect(html).not.toContain("Commit:");
});

it("shows source failures before pinning without claiming prebuilt images", () => {
  const deployment = asTestDouble<EnvironmentDeploymentSummary>()({ sourcePins: {}, buildServiceIds: ["web"], status: "failed", failureCode: "source_acquisition_failed", failureMessage: "Source acquisition failed", createdAt: new Date(), serviceCount: 1 });
  const html = renderToStaticMarkup(createElement(Card, { deployment, progress: null, expanded: true, onExpandedChange() {}, showLogs: false, onLogsChange() {}, actions: null, logsPanel: null }));
  expect(html).toContain("Source acquisition failed");
  expect(html).not.toContain("Using prebuilt images");
});

it("labels a disconnected preparation as unknown, preserving the final diagnosis", () => {
  const deployment = asTestDouble<EnvironmentDeploymentSummary>()({ sourcePins: {}, buildServiceIds: ["web"], status: "failed", failureCode: "sdk_preparation_unknown", failureMessage: "Connection lost; preparation outcome unknown", createdAt: new Date(), serviceCount: 1 });
  const html = renderToStaticMarkup(createElement(Card, { deployment, progress: null, expanded: true, onExpandedChange() {}, showLogs: false, onLogsChange() {}, actions: null, logsPanel: null }));
  expect(html).toContain("Preparation outcome unavailable");
  expect(html).toContain("Connection lost; preparation outcome unknown");
  expect(html).toContain("Preparation ended · outcome unknown");
  expect(html).toContain("Not started");
  expect(html).not.toContain("runtime outcome unknown");
  expect(html).not.toContain("Runtime outcome unavailable");
  expect(html).not.toContain("A complete runtime outcome was not received");
  expect(html).not.toContain("Using prebuilt images");
});

it("shows preparation cancellation without claiming the runtime outcome is unknown", () => {
  const deployment = asTestDouble<EnvironmentDeploymentSummary>()({ sourcePins: {}, buildServiceIds: ["web"], status: "cancelled", createdAt: new Date(), serviceCount: 1 });
  const html = renderToStaticMarkup(createElement(Card, { deployment, progress: null, expanded: true, onExpandedChange() {}, showLogs: false, onLogsChange() {}, actions: null, logsPanel: null }));
  expect(html).toContain("Preparation cancelled");
  expect(html).toContain("Deployment cancelled");
  expect(html).not.toContain("outcome unknown");
  expect(html).not.toContain("Waiting for runtime progress");
});

it.each(["ready", "preview", "success"] as const)("keeps %s preparation completed despite later terminal failure or cancellation", (evidence) => {
  for (const status of ["failed", "cancelled"] as const) {
    const deployment = asTestDouble<EnvironmentDeploymentSummary>()({ sourcePins: {}, buildServiceIds: ["web"], status, createdAt: new Date(), serviceCount: 1,
      deployPreview: evidence === "preview" ? asTestDouble<NonNullable<EnvironmentDeploymentSummary["deployPreview"]>>()({ warnings: [] }) : null,
    });
    const progress = { completed: 0, total: 0, outcome: evidence === "success" ? "success" as const : null, rows: [], compensation: [],
      preparation: { phase: evidence === "ready" ? "ready" as const : "build" as const, serviceId: "web", machineId: "machine", machineName: "builder", message: null },
    };
    const html = renderToStaticMarkup(createElement(Card, { deployment, progress, expanded: true, onExpandedChange() {}, showLogs: false, onLogsChange() {}, actions: null, logsPanel: null }));
    expect(html).toContain("Images prepared");
    expect(html).not.toContain("Image preparation failed");
    expect(html).not.toContain("Preparation cancelled");
  }
});

it("keeps Deploy pending while Git preparation has not reported progress", () => {
  const deployment = asTestDouble<EnvironmentDeploymentSummary>()({ sourcePins: {}, buildServiceIds: ["web"], status: "deploying", createdAt: new Date(), serviceCount: 1 });
  const html = renderToStaticMarkup(createElement(Card, { deployment, progress: null, expanded: true, onExpandedChange() {}, showLogs: false, onLogsChange() {}, actions: null, logsPanel: null }));
  expect(html).toContain("Waiting to prepare images");
  expect(html).not.toContain("Waiting for runtime progress");
});

it("shows service success alongside incomplete logs from terminal evidence", () => {
  const deployment = asTestDouble<EnvironmentDeploymentSummary>()({ sourcePins: {}, buildServiceIds: ["web"], status: "applied", createdAt: new Date(), serviceCount: 1 });
  const progress = { completed: 1, total: 1, outcome: "success" as const, rows: [row("web", "completed")], compensation: [], logsIncomplete: true };
  const html = renderToStaticMarkup(createElement(Card, { deployment, progress, serviceId: "web", expanded: true, onExpandedChange() {}, showLogs: false, onLogsChange() {}, actions: null, logsPanel: null }));
  expect(html).toContain("Service deployed");
  expect(html).toContain("Logs incomplete");
  expect(html).toContain("Images prepared");
  expect(html).not.toContain("Deployment failed");
});
