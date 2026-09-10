// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import { useLiveQuery } from "@tanstack/react-db";
import { createApiCollection } from "#/collections/query-collection";
import { githubReposQueryKey } from "#/modules/github/github.collection";
import { GithubRepositoryRefreshNotice } from "./github-repository-refresh-notice";

afterEach(cleanup);

it("shows failed background refreshes without hiding cached repositories and clears on recovery", async () => {
  const queryClient = new QueryClient();
  const scope = { queryClient, sessionId: "session", userId: "user" };
  let fail = false;
  const raw = createApiCollection({
    queryClient,
    queryKey: githubReposQueryKey(scope),
    queryFn: async () => {
      if (fail) throw new Error("offline");
      return [{ id: "repo", name: "acme/repo" }];
    },
    getKey: (row: { id: string; name: string }) => row.id,
  });
  function Selector() {
    const { data } = useLiveQuery(raw);
    return <><GithubRepositoryRefreshNotice scope={scope} />{data.map((row) => <button key={row.id}>{row.name}</button>)}</>;
  }
  render(<Selector />);
  await screen.findByRole("button", { name: "acme/repo" });
  expect(screen.queryByRole("alert")).toBeNull();
  fail = true;
  await act(async () => { await raw.utils.refetch(); });
  await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("may be out of date"));
  expect(screen.getByRole("button", { name: "acme/repo" })).toBeTruthy();
  fail = false;
  await act(async () => { await raw.utils.refetch(); });
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  cleanup();
  await raw.cleanup();
  queryClient.clear();
});
