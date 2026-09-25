import type { BuildGrantId } from "@ployz/sdk";
import { Schema } from "effect";
import type { CollectorCheckpoint } from "./preparation-progress";

/**
 * Why a Builder is in an Image Build's walk: the Service prefers it, or the Build Order puts it
 * first or next. GitHub records it when it takes the build; a Server's reason comes from the Engine.
 */
export const CANDIDATE_REASONS = ["preferred", "first_in_build_order", "next_in_build_order"] as const;
export type CandidateReason = (typeof CANDIDATE_REASONS)[number];

/** Why a Builder didn't take an Image Build: one entry of its skip trail, in walk order. */
export const skipReasonSchema = Schema.Union([
  Schema.Struct({ builder: Schema.Literal("github"), kind: Schema.Literal("not_connected") }),
  Schema.Struct({ builder: Schema.Literal("github"), kind: Schema.Literals(["no_permission", "no_workflow"]), repository: Schema.String }),
  Schema.Struct({ builder: Schema.Literal("github"), kind: Schema.Literal("multi_platform"), platforms: Schema.Array(Schema.String) }),
  Schema.Struct({ builder: Schema.Literal("github"), kind: Schema.Literal("dispatch_failed"), message: Schema.String }),
  Schema.Struct({ builder: Schema.Literal("github"), kind: Schema.Literal("ended_before_start") }),
  /** It hadn't started the build within its "start within" limit. */
  Schema.Struct({ builder: Schema.Literals(["github", "servers"]), kind: Schema.Literal("not_started"), minutes: Schema.Number }),
]);
export type SkipReason = typeof skipReasonSchema.Type;

const buildGrantIdSchema = Schema.declare<BuildGrantId>(
  (value): value is BuildGrantId => typeof value === "string" && /^[0-9a-f]{64}$/u.test(value),
);

const collectorCheckpointSchema = Schema.Struct({
  build: Schema.Number,
  open: Schema.NullOr(Schema.String),
  stepFailed: Schema.Boolean,
  rows: Schema.Array(Schema.Struct({
    build: Schema.Number, key: Schema.String, name: Schema.String,
    startedAt: Schema.NullOr(Schema.String), completedAt: Schema.NullOr(Schema.String),
    cached: Schema.Boolean, error: Schema.NullOr(Schema.String),
  })),
}) satisfies Schema.Codec<CollectorCheckpoint>;

/**
 * An Image Build GitHub Actions took: its dispatched run and why GitHub took it, then the runner's
 * one check-in and its Build Steps as they arrive. Present exactly while GitHub is the Builder.
 */
export const githubImageBuildSchema = Schema.Struct({
  runId: Schema.Number,
  runUrl: Schema.String,
  /** The `job_workflow_ref` the run's OIDC token must carry: the build workflow on the default branch. */
  workflowRef: Schema.String,
  reason: Schema.Literals(CANDIDATE_REASONS),
  /** When (Unix ms) the runner checked in: the build started. */
  checkedInAt: Schema.NullOr(Schema.Number),
  /** The Build Grant minted at check-in, and the fingerprint the runner was told to build. */
  grant: Schema.NullOr(Schema.Struct({ id: buildGrantIdSchema, fingerprint: Schema.String })),
  /** The runner's Build Steps so far; `platforms` once it reported its end (empty: it failed). */
  report: Schema.NullOr(Schema.Struct({
    received: Schema.Number,
    collector: collectorCheckpointSchema,
    platforms: Schema.NullOr(Schema.Array(Schema.String)),
  })),
});
export type GithubImageBuild = typeof githubImageBuildSchema.Type;
