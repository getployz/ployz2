// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory, createRoute, createRouter, Outlet, RouterProvider } from "@tanstack/react-router";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { DashboardShell } from "./dashboard-shell";
import { ThemeProvider } from "./theme-provider";
import { authClient } from "#/auth/auth-client";
import { Route as RootRoute } from "#/routes/__root";
import { environmentKeys, organizationKeys, projectKeys } from "#/modules/environment-design/workspace-queries";

const clients: QueryClient[] = [];
const mediaListeners = new Map<(event: MediaQueryListEvent) => void, string>();
const scrollSelector = '[data-scroll-restoration-id="wireframe-content"]';
const originalScrollTo = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollTo");
beforeEach(() => {
  vi.stubGlobal("innerWidth", 1200);
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: query.includes("max-width") && window.innerWidth <= 860,
    addEventListener(_type: string, listener: (event: MediaQueryListEvent) => void) { mediaListeners.set(listener, query); },
    removeEventListener(_type: string, listener: (event: MediaQueryListEvent) => void) { mediaListeners.delete(listener); },
  }));
  vi.stubGlobal("fetch", async () => Response.json(null));
  const session = authClient.$store.atoms["session"];
  if (!session) throw new Error("Auth session store unavailable");
  session.set({ ...session.get(), data: null, isPending: false, isRefetching: false });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("scrollTo", () => {});
  Object.defineProperty(HTMLElement.prototype, "scrollTo", {
    configurable: true,
    value({ top = 0, left = 0 }: ScrollToOptions) { this.scrollTop = top; this.scrollLeft = left; },
  });
});
afterEach(() => {
  cleanup();
  for (const client of clients.splice(0)) client.clear();
  mediaListeners.clear(); vi.clearAllMocks(); vi.unstubAllGlobals();
  if (originalScrollTo) Object.defineProperty(HTMLElement.prototype, "scrollTo", originalScrollTo);
  else Reflect.deleteProperty(HTMLElement.prototype, "scrollTo");
});

async function show() {
  const client = new QueryClient({ defaultOptions: { queries: { enabled: false, retry: false, staleTime: Infinity } } });
  clients.push(client);
  const environmentData = { id: "production", projectId: "project", namespace: "production", name: "Production" };
  client.setQueryData(organizationKeys.state("acme"), { activeOrganization: { id: "org", slug: "acme", name: "Acme" }, organizations: [{ id: "org", slug: "acme", name: "Acme" }] });
  client.setQueryData(projectKeys.list("acme"), [{ id: "project", slug: "store", name: "Store", resolvedEnvironment: environmentData }]);
  client.setQueryData(environmentKeys.detail("acme", "store", "production"), environmentData);
  for (const table of ["project", "environment", "service", "environment_resource", "resource_lineage", "environment_canvas_node_position", "environment_node_config_snapshot", "volume_remove_attempt"]) {
    client.setQueryData(["collections", "test-session", "test-user", "acme", table], table === "environment" ? [environmentData] : []);
  }
  RootRoute.updateLoader({ loader: () => ({ theme: "light", session: null }) });
  const root = RootRoute.update({ component: () => <ThemeProvider theme="light"><Outlet /></ThemeProvider> });
  const protectedRoute = createRoute({ getParentRoute: () => root, id: "_protected", beforeLoad: () => ({ session: { session: { id: "test-session" }, user: { id: "test-user" } } }) });
  const cloud = createRoute({ getParentRoute: () => protectedRoute, path: "cloud" });
  const organization = createRoute({ getParentRoute: () => cloud, path: "$organizationSlug" });
  const projectLayout = createRoute({ getParentRoute: () => organization, id: "_project" });
  const project = createRoute({ getParentRoute: () => projectLayout, path: "$projectSlug" });
  const environment = createRoute({ getParentRoute: () => project, path: "$environmentSlug", component: () => (
    <DashboardShell scope={{ kind: "environment", organizationSlug: "acme", projectSlug: "store", environmentSlug: "production" }}><Outlet /></DashboardShell>
  ) });
  const logs = createRoute({ getParentRoute: () => environment, path: "logs", component: () => <div>Log entries</div> });
  const settings = createRoute({ getParentRoute: () => environment, path: "settings", component: () => <div>Environment preferences</div> });
  const router = createRouter({
    context: { queryClient: client },
    routeTree: root.addChildren([protectedRoute.addChildren([cloud.addChildren([organization.addChildren([projectLayout.addChildren([project.addChildren([environment.addChildren([logs, settings])])])])])])]),
    history: createMemoryHistory({ initialEntries: ["/cloud/acme/store/production/logs"] }),
    scrollRestoration: true, scrollToTopSelectors: [scrollSelector],
  });
  await router.load();
  const view = render(<QueryClientProvider client={client}><RouterProvider router={router} /></QueryClientProvider>);
  await screen.findByRole("heading", { name: "Logs" });
  await waitFor(() => expect(screen.getAllByRole("button", { name: "Project and environment: Store / Production" }).length).toBeGreaterThan(0));
  return { ...view, router };
}

it("renders real scope queries and retains named navigation when collapsed", async () => {
  const { router } = await show();
  const sidebar = screen.getByRole("complementary", { name: "Dashboard navigation" });
  expect(within(sidebar).getByRole("button", { name: "Organization: Acme" })).toBeTruthy();
  fireEvent.click(within(sidebar).getByRole("button", { name: "Collapse sidebar" }));
  expect(within(sidebar).getByRole("button", { name: "Expand sidebar" })).toBeTruthy();
  expect(within(sidebar).queryByRole("link", { name: "Ployz home" })).toBeNull();
  expect(within(sidebar).getByRole("button", { name: "Architecture" })).toBeTruthy();
  expect(within(sidebar).getByRole("link", { name: "Logs" }).getAttribute("aria-current")).toBe("page");
  expect(router.state.location.pathname).toBe("/cloud/acme/store/production/logs");
  fireEvent.click(within(sidebar).getByRole("button", { name: "Expand sidebar" }));
  expect(within(sidebar).getByRole("link", { name: "Ployz home" })).toBeTruthy();
});

it("resets its persistent scroll surface when navigating to a different environment page", async () => {
  const { container, router } = await show();
  const surface = container.querySelector<HTMLElement>(scrollSelector);
  expect(surface).not.toBeNull();
  if (!surface) throw new Error("Dashboard scroll surface not found");
  surface.scrollTop = 640; fireEvent.scroll(surface);
  await act(async () => { router.history.push("/cloud/acme/store/production/settings"); });
  await screen.findByRole("heading", { name: "Settings" });
  expect(container.querySelector(scrollSelector)).toBe(surface);
  await waitFor(() => expect(surface.scrollTop).toBe(0));
});

it("opens the directory from a sibling page without navigating away", async () => {
  const { router } = await show();
  const sidebar = screen.getByRole("complementary", { name: "Dashboard navigation" });
  fireEvent.click(within(sidebar).getByRole("button", { name: "Expand Architecture" }));
  expect(await within(sidebar).findByText("No resources yet")).toBeTruthy();
  expect(router.state.location.pathname).toBe("/cloud/acme/store/production/logs");
  fireEvent.click(within(sidebar).getByRole("button", { name: "Collapse Architecture" }));
  await waitFor(() => expect(within(sidebar).queryByText("No resources yet")).toBeNull());
});

it("uses the mobile picker for the same scoped destinations and closes it after selection", async () => {
  vi.stubGlobal("innerWidth", 390);
  const { router } = await show();
  const main = screen.getByRole("main");
  expect(within(main).getByRole("button", { name: "Organization: Acme" })).toBeTruthy();
  fireEvent.click(within(main).getByRole("button", { name: "Project navigation" }));
  const picker = await screen.findByRole("dialog", { name: "Project navigation" });
  const settings = within(picker).getByRole("link", { name: "Settings" });
  expect(settings.getAttribute("href")).toBe("/cloud/acme/store/production/settings");
  fireEvent.click(settings);
  await waitFor(() => expect(router.state.location.pathname).toBe("/cloud/acme/store/production/settings"));
  await waitFor(() => expect(screen.queryByRole("dialog", { name: "Project navigation" })).toBeNull());
  expect(within(main).getByRole("button", { name: "Project navigation" }).textContent).toContain("Settings");
});

it("removes an open collapsed-rail popup when resizing to mobile", async () => {
  const { router } = await show();
  fireEvent.click(screen.getByRole("button", { name: "Collapse sidebar" }));
  fireEvent.click(screen.getByRole("button", { name: "Architecture" }));
  expect(await screen.findByRole("dialog", { name: "Architecture" })).toBeTruthy();
  await act(async () => {
    vi.stubGlobal("innerWidth", 390);
    for (const [listener, query] of mediaListeners) {
      listener(Object.assign(new Event("change"), { matches: query.includes("max-width"), media: query }));
    }
  });
  await waitFor(() => expect(screen.queryByRole("dialog", { name: "Architecture" })).toBeNull());
  expect(screen.queryByRole("complementary", { name: "Dashboard navigation" })).toBeNull();
  expect(screen.getByRole("button", { name: "Project navigation" })).toBeTruthy();
  expect(router.state.location.pathname).toBe("/cloud/acme/store/production/logs");
});
