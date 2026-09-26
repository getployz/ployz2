import "@tanstack/react-start/server-only";
import { eq } from "drizzle-orm";
import { Effect } from "effect";
import { asRecord, asString } from "#/lib/json";
import { canonicalJson } from "#/modules/environment-design/canonical-json";
import { environmentNodeConfigSnapshot } from "#/modules/runtime/tables";
import { Database } from "#/server/database.server";
import type { AttemptTargetNodes } from "./deployment-contract";
import { loadEnvironmentSnapshotProjection } from "./environment-state.repository.server";
import { environmentDeployment } from "./tables";

type Node = { nodeType: "service" | "volume"; nodeId: string; config: unknown };

// Raw JSON reads: an applied config may predate today's config schema.
const nameOf = ({ nodeType, config }: Node) => asString(asRecord(config)?.[nodeType === "service" ? "privateDns" : "name"]) ?? "";
const needsBuild = ({ nodeType, config }: Node) => nodeType === "service" && asRecord(asRecord(config)?.["source"])?.["type"] === "git";

/**
 * Freezes the attempt's target node list: each of its snapshots diffed against the environment's Applied State,
 * plus the applied nodes it drops (Removed). Admission writes it, and the attempt's start rewrites it against
 * Applied State at that moment. Runs inside the caller's transaction, which holds the environment's queue lock.
 */
export const freezeAttemptTargetNodes = Effect.fn("Deployments.freezeAttemptTargetNodes")(function* (
  input: { environmentId: string; environmentDeploymentId: string },
) {
  const { drizzle } = yield* Database;
  const [target, projection] = yield* Effect.all([
    drizzle.select({ nodeType: environmentNodeConfigSnapshot.nodeType, nodeId: environmentNodeConfigSnapshot.nodeId, config: environmentNodeConfigSnapshot.config })
      .from(environmentNodeConfigSnapshot).where(eq(environmentNodeConfigSnapshot.environmentDeploymentId, input.environmentDeploymentId)),
    loadEnvironmentSnapshotProjection({ kind: "environment", environmentId: input.environmentId }),
  ]);
  const applied = projection.appliedSavedNodeByKey;
  const targetKeys = new Set(target.map((node) => `${node.nodeType}:${node.nodeId}`));
  const targetNodes: AttemptTargetNodes = {
    version: 1,
    nodes: [
      ...target.map((node) => {
        const before = applied.get(`${node.nodeType}:${node.nodeId}`);
        const build = needsBuild(node);
        const changed = build || !before || canonicalJson(before.config) !== canonicalJson(node.config);
        return { nodeId: node.nodeId, nodeType: node.nodeType, name: nameOf(node), changed, removed: false, needsBuild: build };
      }),
      ...[...applied].filter(([key]) => !targetKeys.has(key)).map(([, node]) => (
        { nodeId: node.nodeId, nodeType: node.nodeType, name: nameOf(node), changed: true, removed: true, needsBuild: false })),
    ],
  };
  yield* drizzle.update(environmentDeployment).set({ targetNodes })
    .where(eq(environmentDeployment.id, input.environmentDeploymentId));
});
