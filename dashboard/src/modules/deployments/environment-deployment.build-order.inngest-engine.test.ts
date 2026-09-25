import { useServiceFreeEffectRunner } from "#/test/service-free-effect-runner";
import { InngestTestEngine, mockCtx } from "@inngest/test";
import { Effect } from "effect";
import { Inngest } from "inngest";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { asTestDouble } from "#/lib/test-double";
import type { BuildCandidate } from "#/modules/deployments/build-order";
import * as buildOrder from "#/modules/deployments/build-order.server";
import * as githubImageBuilds from "#/modules/deployments/github-image-builds.server";
import * as imageBuilds from "#/modules/deployments/image-builds.server";
import type { ImageBuildAttempt, ImageBuildOutcome, ImageBuildTarget } from "#/modules/deployments/image-builds.server";
import * as runtimeActivities from "#/modules/deployments/runtime-activities.server";
import * as runtimeHydration from "#/modules/deployments/runtime-hydration.repository.server";
import * as runtimeLifecycle from "#/modules/deployments/runtime-lifecycle.repository.server";
import type { DeploymentContext } from "#/modules/deployments/runtime-repository.server";
import { createProcessEnvironmentDeployment } from "./environment-deployment.inngest";

/**
 * Walking the Build Order, with each Builder faked at its activity: which Builder ends up with the
 * build, what each is allowed to wait, and where the walk stops.
 */

const target: ImageBuildTarget = { id: "build-api", deploymentId: "deployment-1", serviceId: "service-api", image: "api", buildIndex: 0 };
const settled = (status: "built" | "failed"): ImageBuildAttempt => ({ kind: "settled", result: { imageBuildId: target.id, image: "api", status } });
const skipped = (reason: string): ImageBuildAttempt => ({ kind: "skipped", reason });
const RUN_ID = 77;

const fake = {
  candidates: [] as BuildCandidate[],
  /** Each server go: what it returns, and the start limit it was given. */
  servers: [] as ImageBuildAttempt[],
  serverLimits: [] as (number | undefined)[],
  serverPreferred: [] as (string | undefined)[],
  githubStart: null as githubImageBuilds.GithubBuildStart | null,
  /** The Workflow run webhook: the run's completion, or nothing before the wait's timeout. */
  runCompletes: [] as boolean[],
  waits: [] as (string | number | undefined)[],
  answered: new Map<string, boolean>(),
  withdraw: null as ImageBuildAttempt | { kind: "started" } | null,
  finish: [] as ImageBuildAttempt[],
  finishTimedOut: [] as boolean[],
  failed: [] as string[],
  deployed: 0,
};

useServiceFreeEffectRunner();
vi.spyOn(runtimeLifecycle, "recordInngestRun").mockImplementation(() => Effect.succeed(true));
vi.spyOn(runtimeLifecycle, "beginEnvironmentDeploymentPlanning").mockImplementation(() => Effect.succeed({ state: "started" }));
vi.spyOn(runtimeLifecycle, "ownsDeploymentRun").mockImplementation(() => Effect.succeed(true));
vi.spyOn(runtimeLifecycle, "markDeploymentFailedIfOwned").mockImplementation(() => Effect.succeed(true));
const context = {
  deployment: { id: "deployment-1", environmentId: "environment-1", status: "queued", inngestRunId: null },
  environment: { id: "environment-1", namespace: "production" },
  project: { id: "project-1", organizationId: "organization-1" },
  organization: { id: "organization-1", slug: "organization" },
  snapshots: [], appliedServiceIds: [], volumes: [],
} satisfies DeploymentContext;
const loadContext = vi.fn();
loadContext.mockImplementation(async () => fake.deployed ? { ...context, deployment: { ...context.deployment, status: "applied" } } : context);
vi.spyOn(runtimeHydration, "loadDeploymentContext").mockImplementation(() => Effect.promise(() => loadContext()));
vi.spyOn(runtimeActivities, "executeLatestEnvironmentDeployment").mockImplementation(() => Effect.sync(() => {
  fake.deployed += 1;
  return { type: "success", completed: 0 } as const;
}));
vi.spyOn(imageBuilds, "startImageBuilds").mockImplementation(() => Effect.succeed([target]));
vi.spyOn(buildOrder, "imageBuildCandidates").mockImplementation(() => Effect.sync(() => fake.candidates));
vi.spyOn(runtimeActivities, "executeImageBuild").mockImplementation((_build, startWithinMs, preferredMachine) => Effect.sync(() => {
  fake.serverLimits.push(startWithinMs);
  fake.serverPreferred.push(preferredMachine);
  return fake.servers.shift() ?? settled("built");
}));
vi.spyOn(githubImageBuilds, "startGithubImageBuild").mockImplementation(() => Effect.sync(() => fake.githubStart ?? { kind: "dispatched", runId: RUN_ID }));
vi.spyOn(githubImageBuilds, "withdrawGithubImageBuild").mockImplementation(() => Effect.sync(() => fake.withdraw ?? { kind: "started" }));
vi.spyOn(githubImageBuilds, "finishGithubImageBuild").mockImplementation((_build, timedOut) => Effect.sync(() => {
  fake.finishTimedOut.push(timedOut);
  return fake.finish.shift() ?? settled("built");
}));
vi.spyOn(imageBuilds, "settleImageBuildResult").mockImplementation((build, outcome: ImageBuildOutcome) => Effect.sync(() => {
  if (outcome.status === "failed") fake.failed.push(outcome.message);
  return { imageBuildId: build.id, image: build.image, status: outcome.status };
}));

function walk() {
  return new InngestTestEngine({
    function: createProcessEnvironmentDeployment(new Inngest({ id: "build-order" })),
    events: [{ name: "environment/deploy.requested", data: { environmentDeploymentId: "deployment-1", environmentId: "environment-1" } }],
    transformCtx: (ctx) => {
      const mocked = mockCtx(ctx);
      // @inngest/test can't mock waitForEvent (inngest 4 validates its lazy promise as an event).
      const waitForEvent = async (id: string, options: { timeout?: string | number }) => {
        // The replaced tool isn't memoized, and the engine replays: answer each wait once.
        if (!fake.answered.has(id)) {
          fake.waits.push(options.timeout);
          fake.answered.set(id, fake.runCompletes.shift() ?? false);
        }
        return fake.answered.get(id) ? { name: "github/build-run.completed", data: { runId: RUN_ID } } : null;
      };
      return { ...mocked, step: { ...mocked.step, waitForEvent: asTestDouble<typeof mocked.step.waitForEvent>()(waitForEvent) } };
    },
  });
}

/** Runs the attempt: whether it failed, and whether it went on to deploy. */
const outcome = async () => {
  const output = await walk().execute();
  return { error: output.error, deployed: fake.deployed > 0 };
};

describe("walking the Build Order", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Object.assign(fake, {
      candidates: [], servers: [], serverLimits: [], serverPreferred: [], githubStart: null, runCompletes: [], waits: [], answered: new Map(),
      withdraw: null, finish: [], finishTimedOut: [], failed: [], deployed: 0,
    });
  });

  it("gives a lone Builder no start limit: your servers only waits in the queue", async () => {
    fake.candidates = [{ builder: "servers" }];
    expect(await outcome()).toMatchObject({ deployed: true });
    expect(new Set(fake.serverLimits)).toEqual(new Set([undefined]));
  });

  it("hands a build the servers didn't start in time to GitHub, whose last place waits for the run", async () => {
    fake.candidates = [{ builder: "servers" }, { builder: "github" }];
    fake.servers = [skipped("Your servers: none started it in 3 min")];
    fake.runCompletes = [true];
    expect(await outcome()).toMatchObject({ deployed: true });
    expect(fake.serverLimits[0]).toBe(imageBuilds.START_WITHIN_MS);
    expect(fake.waits).toEqual(["2h"]);
  });

  it("falls to the servers, which then wait, when no GitHub runner checks in within the limit", async () => {
    fake.candidates = [{ builder: "github" }, { builder: "servers" }];
    fake.runCompletes = [false];
    fake.withdraw = skipped("GitHub: no runner in 3 min");
    expect(await outcome()).toMatchObject({ deployed: true });
    expect(fake.waits[0]).toBe("3m");
    expect(new Set(fake.serverLimits)).toEqual(new Set([undefined]));
  });

  it("skips GitHub at once when it can't take the build", async () => {
    fake.candidates = [{ builder: "github" }, { builder: "servers" }];
    fake.githubStart = skipped("GitHub: needs amd64+arm64");
    expect(await outcome()).toMatchObject({ deployed: true });
    expect(fake.waits).toEqual([]);
    expect(fake.serverLimits.length).toBeGreaterThan(0);
  });

  it("moves on when the Workflow run webhook reports the run ended before it checked in", async () => {
    fake.candidates = [{ builder: "github" }, { builder: "servers" }];
    fake.runCompletes = [true];
    fake.finish = [skipped("GitHub: the run ended before it started")];
    expect(await outcome()).toMatchObject({ deployed: true });
    expect(fake.finishTimedOut[0]).toBe(false);
    expect(fake.serverLimits.length).toBeGreaterThan(0);
  });

  it("keeps a GitHub run that checked in before the limit, and never moves it when it fails", async () => {
    fake.candidates = [{ builder: "github" }, { builder: "servers" }];
    fake.runCompletes = [false, true];
    fake.withdraw = { kind: "started" };
    fake.finish = [settled("failed")];
    const result = await outcome();
    expect(result).toMatchObject({ deployed: false, error: expect.objectContaining({ message: "Image Build failed: api." }) });
    expect(fake.waits).toEqual(["3m", "2h"]);
    expect(fake.serverLimits).toEqual([]);
  });

  it("never moves a server build that started and failed", async () => {
    fake.candidates = [{ builder: "servers" }, { builder: "github" }];
    fake.servers = [settled("failed")];
    expect(await outcome()).toMatchObject({ deployed: false });
    expect(githubImageBuilds.startGithubImageBuild).not.toHaveBeenCalled();
  });

  it("asks the Cluster for a Preferred Server first, then walks the Build Order without your servers", async () => {
    const preferred = "a".repeat(32);
    fake.candidates = [{ builder: "servers", machineId: preferred }, { builder: "github" }];
    fake.servers = [skipped("Your servers: none started it in 3 min")];
    fake.runCompletes = [true];
    expect(await outcome()).toMatchObject({ deployed: true });
    expect(fake.serverLimits[0]).toBe(imageBuilds.START_WITHIN_MS);
    expect(new Set(fake.serverPreferred)).toEqual(new Set([preferred]));
    expect(fake.waits).toEqual(["2h"]);
  });

  it("fails the build with the last skip's reason when every Builder was skipped", async () => {
    fake.candidates = [{ builder: "github" }];
    fake.githubStart = skipped("GitHub: no workflow in acme/api");
    expect(await outcome()).toMatchObject({ deployed: false, error: expect.objectContaining({ message: "Image Build failed: api." }) });
    expect(fake.failed).toContain("GitHub: no workflow in acme/api");
  });
});
