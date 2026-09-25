/* oxlint-disable -- PROTOTYPE: throwaway fake-data UI on prototype/build-order-dashboard, never merged. */
// PROTOTYPE (prototype/build-order-dashboard): fake Build Order state, no backend.
import { useSyncExternalStore } from "react";

export type BuildOrder =
  | "github-then-servers"
  | "servers-then-github"
  | "servers-only"
  | "github-only";

export const BUILD_ORDER_LABELS: Record<BuildOrder, string> = {
  "github-then-servers": "GitHub, then servers",
  "servers-then-github": "Servers, then GitHub",
  "servers-only": "Servers only",
  "github-only": "GitHub only",
};


export type FakeServer = {
  id: string;
  name: string;
  runsServices: boolean;
  builds: boolean;
  concurrency: number | null; // null = automatic
  ramGb: number;
  buildingNow: string[];
  cacheFor: string[];
};

export type RepoReadiness = "ready" | "setup-needed" | "pr-open";

export type FakeRepo = {
  fullName: string;
  services: string[];
  readiness: RepoReadiness;
};

type State = {
  buildOrder: BuildOrder;
  servers: FakeServer[];
  repos: FakeRepo[];
  preferredBuilder: Record<string, string | undefined>; // "github" or a server id; absent = Auto
};

let state: State = {
  buildOrder: "github-then-servers",
  servers: [
    { id: "hel-1", name: "hel-1", runsServices: true, builds: true, concurrency: null, ramGb: 64, buildingNow: [], cacheFor: ["worker"] },
    { id: "hel-2", name: "hel-2", runsServices: true, builds: false, concurrency: null, ramGb: 64, buildingNow: [], cacheFor: [] },
    { id: "nuc-basement", name: "nuc-basement", runsServices: false, builds: true, concurrency: null, ramGb: 32, buildingNow: ["web"], cacheFor: ["web", "api"] },
  ],
  repos: [
    { fullName: "acme/shop", services: ["web", "worker"], readiness: "ready" },
    { fullName: "acme/api", services: ["api"], readiness: "setup-needed" },
  ],
  preferredBuilder: {},
};

const listeners = new Set<() => void>();

export function update(recipe: (draft: State) => State) {
  state = recipe(state);
  for (const listener of listeners) listener();
}

export function useBuildOrderState(): State {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => state,
    () => state,
  );
}

export function automaticConcurrency(server: FakeServer): number {
  if (server.runsServices) return 1;
  return Math.min(4, Math.max(1, Math.floor(server.ramGb / 4)));
}
