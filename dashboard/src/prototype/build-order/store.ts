/* oxlint-disable -- PROTOTYPE: throwaway fake-data UI on prototype/build-order-dashboard, never merged. */
// PROTOTYPE (prototype/build-order-dashboard): fake Build Order state, no backend.
import { useSyncExternalStore } from "react";

export type BuildOrder =
  | "github-then-servers"
  | "servers-then-github"
  | "servers-only"
  | "github-only";

export const BUILD_ORDER_LABELS: Record<BuildOrder, string> = {
  "github-then-servers": "GitHub Actions, then your servers",
  "servers-then-github": "Your servers, then GitHub Actions",
  "servers-only": "Your servers only",
  "github-only": "GitHub Actions only",
};

export const BUILD_ORDER_DESCRIPTIONS: Record<BuildOrder, string> = {
  "github-then-servers":
    "Builds run on GitHub first. If no runner starts within 3 minutes, your servers build it.",
  "servers-then-github":
    "Your servers build first. If they are all busy for 3 minutes, GitHub Actions takes it.",
  "servers-only": "Source and build secrets never leave your servers.",
  "github-only": "Nothing builds on your servers. Fails if no runner starts in time.",
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
  serviceOverrides: Record<string, BuildOrder | undefined>;
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
  serviceOverrides: {},
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
