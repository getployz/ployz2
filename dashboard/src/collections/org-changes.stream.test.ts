import { QueryClient } from "@tanstack/react-query";
import { expect, it } from "vitest";
import { changeCollections } from "#/collections/collections";
import { applyOrganizationChanges } from "#/collections/org-changes.stream";
import { organizationKeys } from "#/modules/environment-design/workspace.queries";
import { orgStoreSeed, orgStoreTableNames } from "#/test/org-store-tables";

it("refetches only the named collections and re-reads a renamed organization's state", async () => {
  const queryClient = new QueryClient();
  const scope = { queryClient, sessionId: "session", userId: "user" };
  for (const table of orgStoreTableNames) queryClient.setQueryData(["collections", "session", "user", "acme", table], orgStoreSeed([]));
  queryClient.setQueryData(organizationKeys.state("acme"), { name: "Acme" });
  const active = Object.values(changeCollections).map((get) => get("acme", scope).subscribeChanges(() => {}));
  const fetched: unknown[] = [];
  queryClient.getQueryCache().subscribe((event) => {
    if (event.type === "updated" && event.action.type === "fetch") fetched.push(event.query.queryKey.at(-1));
  });

  applyOrganizationChanges(["environment_deployment", "organization"], "acme", scope);

  expect(fetched).toEqual(["environment_deployment"]);
  expect(queryClient.getQueryState(organizationKeys.state("acme"))?.isInvalidated).toBe(true);
  for (const subscription of active) subscription.unsubscribe();
});
