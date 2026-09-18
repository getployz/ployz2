import { projectEnvironmentChanges, type ReviewChangeSet, type ReviewNodeChange } from "@ployz/sdk/config";
import type { EnvironmentResourceNodeConfigByType } from "./environment-resource-node";
import type { ServiceDeploymentConfig } from "./services";

export type EnvironmentNodeLifecycle = "create" | "update" | "delete" | "none";
export type EnvironmentNodeIdentity = { type: "service" | "variable_group" | "volume"; id: string };
export type EnvironmentNodeConfigByType = { service: ServiceDeploymentConfig } & EnvironmentResourceNodeConfigByType;
export type EnvironmentNodeProjection = {
  [T in EnvironmentNodeIdentity["type"]]: {
    node: EnvironmentNodeIdentity & { type: T };
    config: EnvironmentNodeConfigByType[T] | null;
  };
}[EnvironmentNodeIdentity["type"]];
export type EnvironmentNodeIntroductionProjection = {
  [T in EnvironmentNodeIdentity["type"]]: {
    node: EnvironmentNodeIdentity & { type: T };
    config: EnvironmentNodeConfigByType[T];
  };
}[EnvironmentNodeIdentity["type"]];
export type EnvironmentStateProjection = { token: string; nodes: EnvironmentNodeProjection[] };
export type EnvironmentNodeIntroductionsProjection = { token: string; nodes: EnvironmentNodeIntroductionProjection[] };
/** Head is `submitted ?? applied`; a node absent from Head compares against its Introduction. Core owns that rule. */
export type EnvironmentChangeSetProjectionInput = {
  working: EnvironmentStateProjection;
  applied: EnvironmentStateProjection;
  submitted: EnvironmentStateProjection | null;
  nodeIntroductions: EnvironmentNodeIntroductionsProjection;
};

export function buildEnvironmentChangeSet(input: EnvironmentChangeSetProjectionInput): ReviewChangeSet {
  return projectEnvironmentChanges(input);
}

/** The change group for one node, computed by the same rule as the whole set. */
export function buildEnvironmentNodeChange(input: {
  working: EnvironmentNodeProjection;
  applied: EnvironmentNodeProjection[];
  submitted: EnvironmentNodeProjection[] | null;
  introduction: EnvironmentNodeIntroductionProjection | null;
}): ReviewNodeChange | null {
  const only = (nodes: EnvironmentNodeProjection[]) =>
    nodes.filter(node => node.node.type === input.working.node.type && node.node.id === input.working.node.id);
  return projectEnvironmentChanges({
    working: { token: "working", nodes: [input.working] },
    applied: { token: "applied", nodes: only(input.applied) },
    submitted: input.submitted ? { token: "submitted", nodes: only(input.submitted) } : null,
    nodeIntroductions: { token: "introductions", nodes: input.introduction ? [input.introduction] : [] },
  }).groups[0] ?? null;
}
