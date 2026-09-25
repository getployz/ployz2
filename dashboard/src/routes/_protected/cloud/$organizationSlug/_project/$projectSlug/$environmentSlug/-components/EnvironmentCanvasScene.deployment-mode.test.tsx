// @vitest-environment jsdom
import { Suspense } from "react";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { DbProvider } from "@tanstack/react-db";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory, createRootRoute, createRoute, createRouter, Outlet, RouterProvider,
} from "@tanstack/react-router";
import { Schema } from "effect";
import { parseServiceConfig } from "@ployz/sdk/config";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getEnvironmentDeploymentsCollection, getEnvironmentSavedStateRevisionsCollection } from "#/collections/collections";
import { orgStoreOptions } from "#/collections/org-store";
import { preloadCollection } from "#/collections/query-collection";
import { getDbClient } from "#/collections/scope";
import type { DeploymentProgress, DeploymentProgressRow } from "#/modules/deployments/deployment-progress";
import { environmentChangeStateOptions } from "#/modules/deployments/environment-change-state.queries";
import { defaultServicePolicy } from "#/modules/environment-design/service-policy";
import { RuntimeProvider } from "#/providers/runtime-provider";
import { orgStoreSeed, orgStoreTableNames } from "#/test/org-store-tables";
import { serviceSearchSchema } from "../services/$serviceId/-components/service-pages";
import { canvasRouteSearch } from "./deployment-mode";
import { EnvironmentCanvasScene } from "./EnvironmentCanvasScene";
import { ENVIRONMENT_INDEX_ROUTE_TO, ENVIRONMENT_SERVICE_ROUTE_TO } from "./environment-route-paths";
import { Route as deploymentsRoute } from "../deployments";

const organizationId = "00000000-0000-4000-8000-000000000001";
const projectId = "00000000-0000-4000-8000-000000000002";
const environmentId = "00000000-0000-4000-8000-000000000003";
const [previous, attemptId, failedId, runningId] = ["a0000000-0000-4000-8000-000000000011", "b0000000-0000-4000-8000-000000000012",
  "c0000000-0000-4000-8000-000000000013", "d0000000-0000-4000-8000-000000000014"];
const [api, old, worker, web] = ["00000000-0000-4000-8000-000000000021", "00000000-0000-4000-8000-000000000022", "00000000-0000-4000-8000-000000000023", "00000000-0000-4000-8000-000000000024"];
const params = { organizationSlug: "acme", projectSlug: "shop", environmentSlug: "production" };
const createdAt = new Date("2026-09-01T00:00:00Z");
const config = (privateDns: string) => parseServiceConfig({ version: 2, source: { version: 1, type: "image", image: "nginx:1", credentials: { type: "none" } },
  healthcheck: { type: "none" }, restartPolicy: "unless-stopped", privateDns });
const intentService = (id: string, slug: string) => {
  const { env: _env, mounts: _mounts, ...authored } = config(slug);
  return { id, lineageId: id, slug, config: authored, variables: [], volumeAttachments: [] };
};
const service = (id: string, name: string) => ({ id, organizationId, projectId, environmentId, lineageId: id, name, policy: defaultServicePolicy,
  hasRegistryCredential: false, firstDeployedAt: createdAt, createdAt, updatedAt: createdAt });
const deployment = (id: string, minute: number, runtimeProgress: DeploymentProgress | null, status = "applied") => ({
  id, organizationId, environmentId, triggerOrigin: { origin: "manual", actorId: "user" }, savedStateSnapshotId: id, serviceActionPolicy: null,
  status, inngestRunId: null, coreDeployId: null, retryOfDeploymentId: null, sourcePins: {}, variableProducers: null, deployManifest: null,
  deployPreview: null, runtimeProgress, failureCode: null, failureMessage: null, message: null, cancellationRequestedAt: null, dispatchRequestedAt: null,
  startedAt: null, finishedAt: null, createdAt: new Date(createdAt.getTime() + minute * 60_000), updatedAt: createdAt,
});
const snapshot = (deploymentId: string, nodeId: string, privateDns: string) => ({ id: `${deploymentId}:${nodeId}`, organizationId, environmentId,
  environmentDeploymentId: deploymentId, nodeType: "service", nodeId, nodeLineageId: nodeId, configVersion: 1, config: config(privateDns), createdAt, updatedAt: createdAt });
// The attempt removed `old` (its row has no serviceId on the record side), failed `web`'s health check and left `api` unchanged; `worker` came later.
const removal: DeploymentProgressRow = { index: 0, machineId: "m", machineName: "server", serviceId: null, runtimeServiceId: null, serviceName: "old", displayName: null,
  operation: "remove_container", target: null, updateOrder: null, status: "completed", phase: null, elapsedMs: null, deadlineMs: null, health: null, error: null,
  startedAt: 0, finishedAt: 3_000 };
const healthFailure: DeploymentProgressRow = { ...removal, index: 1, serviceId: web, serviceName: "web", operation: "replace_container", status: "failed",
  error: "Health check timed out", containerId: "4e7a19c", startedAt: 3_000, finishedAt: 63_000 };
const rows = new Map<string, unknown[]>(Object.entries({
  project: [{ id: projectId, organizationId, name: "Shop", slug: "shop", createdAt, updatedAt: createdAt }],
  environment: [{ id: environmentId, projectId, organizationId, name: "Production", namespace: "production", createdAt, updatedAt: createdAt,
    intent: { version: 1, environmentSlug: "production", services: [intentService(api, "api"), intentService(web, "web"), intentService(worker, "worker")], volumes: [] } }],
  environment_summary: [{ id: environmentId, projectId, organizationId, name: "Production", namespace: "production", createdAt }],
  service: [service(api, "api"), service(old, "old"), service(web, "web"), service(worker, "worker")],
  environment_deployment: [deployment(previous, 1, null),
    deployment(attemptId, 2, { completed: 1, total: 2, outcome: "failed", rows: [removal, healthFailure], compensation: [] }, "failed")],
  environment_node_config_snapshot: [snapshot(previous, api, "api"), snapshot(previous, old, "old"), snapshot(previous, web, "web"),
    snapshot(attemptId, api, "api"), snapshot(attemptId, web, "web")],
}));
const card = (name: string) => screen.getAllByText(name)[0]?.closest("[data-canvas-node]");

async function openCanvas({ extra = {}, path = "/cloud/acme/shop/production" }: { extra?: Record<string, unknown[]>; path?: string } = {}) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const scope = { queryClient, sessionId: "session", userId: "user" };
  for (const table of orgStoreTableNames) queryClient.setQueryData(["collections", "session", "user", "acme", table], orgStoreSeed([...rows.get(table) ?? [], ...extra[table] ?? []]));
  // The change-state projection stamps its version from these tables, then reads no states.
  await Promise.all([getEnvironmentDeploymentsCollection, getEnvironmentSavedStateRevisionsCollection].map((get) => preloadCollection(get("acme", scope))));
  await queryClient.fetchQuery(environmentChangeStateOptions("acme", scope, async () => []));
  await queryClient.ensureQueryData(orgStoreOptions("acme", scope));

  const root = createRootRoute({ component: Outlet });
  const protectedRoute = createRoute({ getParentRoute: () => root, id: "_protected", component: Outlet,
    beforeLoad: () => ({ session: { session: { id: "session" }, user: { id: "user" } } }) });
  const organization = createRoute({ getParentRoute: () => protectedRoute, path: "cloud/$organizationSlug",
    component: () => <RuntimeProvider organizationSlug="acme"><Outlet /></RuntimeProvider> });
  const projectGroup = createRoute({ getParentRoute: () => organization, id: "_project", component: Outlet });
  const environment = createRoute({ getParentRoute: () => projectGroup, path: "$projectSlug/$environmentSlug",
    loader: () => ({ environmentId, organizationId }), component: Outlet });
  const canvas = createRoute({ getParentRoute: () => environment, id: "_canvas", ...canvasRouteSearch,
    component: () => <Suspense fallback={<p>Loading canvas</p>}><EnvironmentCanvasScene /></Suspense> });
  const index = createRoute({ getParentRoute: () => canvas, path: "/", component: () => null });
  const serviceRoute = createRoute({ getParentRoute: () => canvas, path: "services/$serviceId",
    validateSearch: Schema.toStandardSchemaV1(serviceSearchSchema), component: () => <p>Live service panel</p> });
  const deployments = createRoute({ getParentRoute: () => environment, path: "deployments", beforeLoad: (context) => { deploymentsRoute.options.beforeLoad?.(context as never); } });
  const routeTree = root.addChildren([protectedRoute.addChildren([organization.addChildren([
    projectGroup.addChildren([environment.addChildren([canvas.addChildren([index, serviceRoute]), deployments])]),
  ])])]);
  const router = createRouter({ routeTree, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<DbProvider client={getDbClient(queryClient)}><QueryClientProvider client={queryClient}><RouterProvider router={router} /></QueryClientProvider></DbProvider>);
  await screen.findAllByText("worker");
  return router;
}

const enterDeploymentMode = (router: Awaited<ReturnType<typeof openCanvas>>) =>
  act(() => router.navigate({ to: ENVIRONMENT_INDEX_ROUTE_TO, params, search: { deployment: attemptId } }));

beforeEach(() => {
  vi.stubGlobal("EventSource", class { addEventListener() {} removeEventListener() {} close() {} });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("scrollTo", () => {});
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

describe("deployment mode on the environment canvas", () => {
  it("redraws the canvas as the attempt saw it and returns to live", async () => {
    const router = await openCanvas();
    expect(screen.queryByText("Back to live")).toBeNull();
    expect(screen.queryAllByText("Removed")).toEqual([]);

    await enterDeploymentMode(router);
    expect((await screen.findAllByText("Back to live")).length).toBeGreaterThan(0);
    // Deleted since and removed by the attempt: still drawn. Created afterwards: hidden.
    expect(screen.getAllByText("old")[0]?.closest("[data-canvas-node]")?.textContent).toContain("Removed");
    const unchanged = screen.getAllByText("api")[0]?.closest("[data-canvas-node]");
    expect(unchanged?.textContent).toContain("Unchanged");
    expect(unchanged?.getAttribute("data-dimmed")).toBe("true");
    expect(screen.queryAllByText("worker")).toEqual([]);
    // Read-only: no Create, no change controls, no links into the live service panel.
    expect(screen.queryByRole("button", { name: "Create" })).toBeNull();
    expect(document.querySelector(`a[data-canvas-node="${api}"]`)).toBeNull();

    // Navigating inside the canvas keeps the mode, and the editable live panel stays closed.
    await act(() => router.navigate({ to: ENVIRONMENT_SERVICE_ROUTE_TO, params: { ...params, serviceId: api }, search: {} }));
    expect(router.state.location.search).toMatchObject({ deployment: attemptId });
    expect(screen.queryByText("Live service panel")).toBeNull();

    const [backToLive] = screen.getAllByText("Back to live");
    if (!backToLive) throw new Error("Missing Back to live");
    await act(async () => { fireEvent.click(backToLive); });
    expect((await screen.findAllByText("worker")).length).toBeGreaterThan(0);
    expect(router.state.location.search).not.toHaveProperty("deployment");
    expect(screen.queryAllByText("Removed")).toEqual([]);
  });

  it("shows Build → Deploy and the tail on nodes the attempt changed, and nothing new on live nodes", async () => {
    const router = await openCanvas();
    expect(card("web")?.textContent).not.toContain("Failed");
    expect(document.querySelector("[data-stage], [data-tail]")).toBeNull();

    await enterDeploymentMode(router);
    await screen.findAllByText("Removed");
    const failed = card("web");
    expect(failed?.textContent).toContain("Failed");
    expect(failed?.querySelector('[data-stage="Build"]')?.textContent).toBe("Build —");
    expect(failed?.querySelector('[data-stage="Deploy"]')?.textContent).toBe("Deploy 1m 0s");
    expect(failed?.querySelector("[data-tail]")?.textContent).toBe("server · Health check timed out");
    expect(card("old")?.querySelector('[data-stage="Deploy"]')?.textContent).toBe("Deploy 3s");
    expect(card("old")?.querySelector("[data-tail]")?.textContent).toBe("server · Removing container · done");
    // Unchanged: name and outcome only, dimmed.
    expect(card("api")?.querySelector("[data-stage], [data-tail]")).toBeNull();
    expect(card("api")?.textContent).toBe("apiUnchanged");
  });

  it("leaves the mode on browser Back", async () => {
    const router = await openCanvas();
    await enterDeploymentMode(router);
    await screen.findAllByText("Removed");
    await act(async () => { router.history.back(); });
    expect((await screen.findAllByText("worker")).length).toBeGreaterThan(0);
    expect(screen.queryByText("Back to live")).toBeNull();
  });
});

describe("the deploy bar", () => {
  const bar = () => within(screen.getByRole("group", { name: "Deploy bar" }));
  const click = (element: HTMLElement) => act(async () => { fireEvent.click(element); });

  it("switches between Live and a deployment picked from the list", async () => {
    const router = await openCanvas();
    expect(bar().getByRole("link", { name: "Live" }).getAttribute("data-active")).toBe("true");

    await click(bar().getByRole("button", { name: "Deployments" }));
    expect(router.state.location.search).toMatchObject({ deploymentList: true });
    const list = within(await screen.findByRole("navigation", { name: "Deployments" }));
    // Live first, then deployments newest first.
    expect(list.getAllByRole("link").map((row) => row.textContent)).toEqual([
      expect.stringContaining("Live"), expect.stringContaining("b0000000"), expect.stringContaining("a0000000"),
    ]);

    await click(list.getByRole("link", { name: /b0000000/ }));
    expect(router.state.location.search).toEqual({ deployment: attemptId });
    expect(await screen.findByRole("button", { name: /Deployment b0000000/ })).toBeTruthy();
    expect(screen.queryByRole("navigation", { name: "Deployments" })).toBeNull();

    await click(bar().getByRole("link", { name: "Live" }));
    expect(router.state.location.search).toEqual({});
    expect((await screen.findAllByText("worker")).length).toBeGreaterThan(0);
  });

  it("opens a running attempt directly and offers Cancel, and Retry on a failed one", async () => {
    const router = await openCanvas({ extra: {
      environment_deployment: [deployment(failedId, 3, null, "failed"), deployment(runningId, 4, null, "deploying")],
      environment_node_config_snapshot: [snapshot(runningId, api, "api"), snapshot(runningId, worker, "worker")],
    } });
    await click(bar().getByRole("link", { name: /Deploying 0\/3/ }));
    expect(router.state.location.search).toEqual({ deployment: runningId });
    expect(await bar().findByRole("button", { name: "Cancel" })).toBeTruthy();
    expect(bar().queryByRole("button", { name: "Retry" })).toBeNull();

    await click(bar().getByRole("button", { name: /Deployment d0000000/ }));
    await click(within(await screen.findByRole("navigation", { name: "Deployments" })).getByRole("link", { name: /c0000000/ }));
    expect(await bar().findByRole("button", { name: "Retry" })).toBeTruthy();
    expect(bar().queryByRole("button", { name: "Cancel" })).toBeNull();
  });

  it("stays usable while a service panel is open", async () => {
    const router = await openCanvas();
    await act(() => router.navigate({ to: ENVIRONMENT_SERVICE_ROUTE_TO, params: { ...params, serviceId: api } }));
    await screen.findByText("Live service panel");
    expect(screen.getByRole("group", { name: "Deploy bar" }).closest("[inert]")).toBeNull();
    await click(bar().getByRole("button", { name: "Deployments" }));
    expect(await screen.findByRole("navigation", { name: "Deployments" })).toBeTruthy();
  });

  it("opens the list as a bottom sheet on mobile", async () => {
    vi.stubGlobal("innerWidth", 375);
    await openCanvas();
    await click(bar().getByRole("button", { name: "Deployments" }));
    const sheet = await screen.findByRole("dialog", { name: "Deployments" });
    expect(sheet.getAttribute("data-swipe-direction")).toBe("down");
    expect(within(sheet).getByRole("link", { name: /b0000000/ })).toBeTruthy();
  });

  it("leaves Deployment Mode on Esc", async () => {
    const router = await openCanvas();
    await enterDeploymentMode(router);
    await screen.findAllByText("Removed");
    await act(async () => { fireEvent.keyDown(document.body, { key: "Escape" }); });
    expect(router.state.location.search).toEqual({});
  });

  it("redirects the old Deployments page to the canvas with the list open", async () => {
    const router = await openCanvas({ path: "/cloud/acme/shop/production/deployments" });
    expect(router.state.location.pathname).toBe("/cloud/acme/shop/production");
    expect(router.state.location.search).toEqual({ deploymentList: true });
    expect(await screen.findByRole("navigation", { name: "Deployments" })).toBeTruthy();
  });
});
