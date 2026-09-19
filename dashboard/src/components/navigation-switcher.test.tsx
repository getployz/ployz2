// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory, createRootRoute, createRoute, createRouter, Outlet, RouterProvider } from "@tanstack/react-router";
import { getEnvironmentsCollection } from "#/collections/collections";
import { organizationKeys, projectKeys } from "#/modules/environment-design/workspace-queries";
import { NavigationSwitcher } from "./navigation-switcher";

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("scrollTo", () => {});
  Element.prototype.scrollIntoView ??= () => {};
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

it.each(["desktop", "mobile", "rail"] as const)("offers the same scoped org and combined project/environment choices in %s", async (projection) => {
  const queryClient = new QueryClient({ defaultOptions: { queries: { enabled: false, retry: false, staleTime: Infinity } } });
  const scope = { queryClient, sessionId: "session", userId: "user" };
  for (const organization of ["acme", "other"]) {
    queryClient.setQueryData(organizationKeys.state(organization), {
      activeOrganization: { name: organization === "acme" ? "Acme" : "Other org" },
      organizations: [{ id: "acme", slug: "acme", name: "Acme" }, { id: "other", slug: "other", name: "Other org" }],
    });
    queryClient.setQueryData(projectKeys.list(organization), organization === "acme" ? [
      { id: "project", slug: "store", name: "Store", resolvedEnvironment: { namespace: "production", name: "Production" } },
      { id: "other-project", slug: "docs", name: "Docs", resolvedEnvironment: { namespace: "production", name: "Other production" } },
    ] : []);
    queryClient.setQueryData(["collections", "session", "user", organization, "environment"], organization === "acme" ? [
      { id: "production", projectId: "project", namespace: "production", name: "Production" },
      { id: "staging", projectId: "project", namespace: "staging", name: "Staging" },
      { id: "other-production", projectId: "other-project", namespace: "production", name: "Other production" },
    ] : []);
  }
  const root = createRootRoute({ component: Outlet });
  const protectedRoute = createRoute({ getParentRoute: () => root, id: "_protected", beforeLoad: () => ({ session: { session: { id: "session" }, user: { id: "user" } } }), component: Outlet });
  const organizationRoute = createRoute({ getParentRoute: () => protectedRoute, path: "cloud/$organizationSlug", component: () => <NavigationSwitcher projection={projection} /> });
  const projectGroup = createRoute({ getParentRoute: () => organizationRoute, id: "_project" });
  const environment = createRoute({ getParentRoute: () => projectGroup, path: "$projectSlug/$environmentSlug" });
  const logs = createRoute({ getParentRoute: () => environment, path: "logs" });
  const organizationGroup = createRoute({ getParentRoute: () => organizationRoute, id: "_org" });
  const organizationHome = createRoute({ getParentRoute: () => organizationGroup, path: "~" });
  const routeTree = root.addChildren([protectedRoute.addChildren([organizationRoute.addChildren([projectGroup.addChildren([environment.addChildren([logs])]), organizationGroup.addChildren([organizationHome])])])]);
  const router = createRouter({ routeTree, history: createMemoryHistory({ initialEntries: ["/cloud/acme/store/production/logs?serviceId=old&tab=networking"] }) });
  try {
    render(<QueryClientProvider client={queryClient}><RouterProvider router={router} /></QueryClientProvider>);
    fireEvent.click(await screen.findByRole("button", { name: "Project and environment: Store / Production" }));
    expect(await screen.findByRole("menuitem", { name: "Staging" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Other production" }).getAttribute("href")).toBe("/cloud/acme/docs/production/logs");
    expect(screen.getByRole("menuitem", { name: "Organization" }).getAttribute("href")).toBe("/cloud/acme/~");
    expect(screen.getByRole("menuitem", { name: "New project" }).getAttribute("href")).toBe("/cloud/acme/new");
    expect(screen.getAllByRole("menuitem", { name: "Add environment" })).toHaveLength(2);
    fireEvent.click(screen.getByRole("menuitem", { name: "Staging" }));
    await waitFor(() => expect(router.state.location.href).toBe("/cloud/acme/store/staging/logs"));
    fireEvent.click(screen.getByRole("button", { name: "Organization: Acme" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Other org" }));
    await waitFor(() => expect(router.state.location.href).toBe("/cloud/other/~"));
  } finally {
    cleanup();
    await Promise.all([getEnvironmentsCollection("acme", scope).cleanup(), getEnvironmentsCollection("other", scope).cleanup()]);
    queryClient.clear();
  }
});
