import crypto from "node:crypto";
import { gzipSync } from "node:zlib";
import { InngestTestEngine, mockCtx } from "@inngest/test";
import type { BuildReceipts, Client, MachineDetails, PreparationEvent, PreparationInput, PreparedDeploy } from "@ployz/sdk";
import { eq } from "drizzle-orm";
import { Effect, Layer, Schema } from "effect";
import { Inngest } from "inngest";
import { Header } from "tar";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import * as schema from "#/db/schema";
import { asTestDouble } from "#/lib/test-double";
import { createDefaultServiceHealthcheck, createDefaultServiceRestartPolicy, createGitServiceSource, projectServiceDeploymentConfig } from "#/modules/environment-design/services";
import { GithubApi, type GithubJsonRequest } from "#/modules/github/github-observation.api";
import { GITHUB_OIDC_ISSUER, GithubOidcKeys } from "#/modules/github/github-oidc.server";
import { InngestClient } from "#/modules/inngest/client";
import { makeOrganizationRuntimeLayer, type OrganizationRuntime } from "#/modules/runtime/organization-runtime.server";
import { makePloyzLayer } from "#/modules/runtime/ployz.server";
import { runtimeWatchMachineFixture } from "#/modules/runtime/runtime-watch-frame.test-fixture";
import { AppConfig } from "#/server/config.server";
import type { Database, ReportingDatabase } from "#/server/database.server";
import { makeInngestEffectRunner, type runInngestEffect } from "#/server/run.server";
import { noPairingChanges } from "#/test/organization-runtime";
import { type PostgresTestHarness, startPostgresTestHarness } from "#/test/postgres";
import { makeSecretEncryption, SecretEncryption } from "#/utils/encrypted-secret.server";
import { loadDeploymentBuildLog } from "./deployment-events.server";
import { createMarkCancelledRowBackedWorkflow, createProcessEnvironmentDeployment } from "./environment-deployment.inngest";
import { checkInGithubBuild, recordGithubBuildSteps } from "./github-image-builds.server";

const organizationId = "00000000-0000-4000-8000-000000000801";
const userId = "00000000-0000-4000-8000-000000000802";
const projectId = "00000000-0000-4000-8000-000000000803";
const environmentId = "00000000-0000-4000-8000-000000000804";
const savedId = "00000000-0000-4000-8000-000000000805";
const deploymentId = "00000000-0000-4000-8000-000000000806";
const serviceId = "00000000-0000-4000-8000-000000000807";
const runId = "github-build-run";
const githubRunId = 9001;
const commit = "a".repeat(40);
const pushed = `sha256:${"e".repeat(64)}`;
const workflowRef = "owner/repo/.github/workflows/ployz-build.yml@refs/heads/main";
const encryption = makeSecretEncryption("test-encryption-secret");
const machine = runtimeWatchMachineFixture("b".repeat(32), "entry");

const signing = crypto.generateKeyPairSync("rsa", { modulusLength: 2048 });
const jwk = { ...signing.publicKey.export({ format: "jwk" }), kid: "key-1" };
/** A GitHub Actions OIDC token for the dispatched run, with `claims` overriding any claim. */
type RunnerClaims = { aud: string; repository_id: string; job_workflow_ref: string; run_id: string; event_name: string };
function oidcToken(claims: Partial<RunnerClaims> = {}, key: crypto.KeyObject = signing.privateKey) {
  const part = (json: string) => Buffer.from(json).toString("base64url");
  const body = `${part(JSON.stringify({ alg: "RS256", kid: "key-1" }))}.${part(JSON.stringify({
    iss: GITHUB_OIDC_ISSUER, aud: "http://localhost:3000", exp: Math.floor(Date.now() / 1000) + 300,
    repository_id: "42", job_workflow_ref: workflowRef, run_id: String(githubRunId), event_name: "workflow_dispatch", ...claims,
  }))}`;
  return `${body}.${crypto.sign("RSA-SHA256", Buffer.from(body), key).toString("base64url")}`;
}
type StepsReport = { platforms: string[]; events: { at: number; event: PreparationEvent }[] };
const runnerRequest = (token: string, body?: StepsReport) => new Request("http://localhost:3000/api/builds/x", {
  method: "POST", headers: { authorization: `Bearer ${token}` }, body: body === undefined ? null : JSON.stringify(body),
});

/** GitHub and the entry Machine, faked at their Context boundaries. */
type Fake = {
  github: { operation: string; url: string; body?: unknown }[];
  minted: string[];
  ended: string[];
  prepared: BuildReceipts[];
};

const header = new Header({ path: "root/Dockerfile", size: 0, mode: 0o644, type: "File" });
header.encode();
const archive = gzipSync(Buffer.concat([Buffer.from(header.block ?? Buffer.alloc(512)), Buffer.alloc(1024)]));

function githubApi(fake: Fake) {
  const responses = new Map([
    ["resolve_repository", { id: 42, full_name: "owner/repo", default_branch: "main" }],
    ["fetch_workflow", { path: ".github/workflows/ployz-build.yml", state: "active" }],
    ["dispatch_workflow", { workflow_run_id: githubRunId, html_url: "https://github.com/owner/repo/actions/runs/9001" }],
    ["cancel_run", {}],
  ]);
  return {
    json: <S extends Schema.ConstraintDecoder<unknown>>(request: GithubJsonRequest<S>) => {
      fake.github.push({ operation: request.operation, url: request.url, body: request.body });
      return Schema.decodeUnknownEffect(request.schema)(responses.get(request.operation)).pipe(Effect.orDie);
    },
    archive: () => Effect.succeed(new Response(archive)),
  };
}

function fakeClient(fake: Fake) {
  return asTestDouble<Client>()({
    mintBuildGrant: async ({ repository }: { repository: string }) => {
      fake.minted.push(repository);
      return { id: "f".repeat(64), grant: "ployzgrant1:secret", expires_in_seconds: 3600 };
    },
    endBuildGrant: async ({ id }: { id: string }) => {
      fake.ended.push(id);
      return { pushed };
    },
    inspect: async () => asTestDouble<MachineDetails>()({ id: machine.id }),
    prepare: (input: PreparationInput) => {
      fake.prepared.push(input.build_receipts ?? {});
      const outcome = { type: "success" as const, completed: [] };
      const prepared = asTestDouble<PreparedDeploy>()({
        project_name: "production", operations: [], warnings: [], would_remove: [], preserved_volumes: [],
        buildReceipts: input.build_receipts ?? {}, pruneTargets: [], close: () => undefined,
        confirm: () => ({ abort: () => undefined, finished: Promise.resolve(outcome),
          async *[Symbol.asyncIterator]() { yield { type: "outcome" as const, outcome }; } }),
      });
      return { abort: () => undefined, finished: Promise.resolve(prepared), async *[Symbol.asyncIterator]() { yield* []; } };
    },
    close: async () => undefined,
  });
}

describe("Image Builds on GitHub Actions", () => {
  let harness: PostgresTestHarness;
  let fake: Fake;

  beforeAll(async () => {
    harness = await startPostgresTestHarness();
  }, 60_000);
  afterAll(async () => {
    await harness?.stop();
  });

  beforeEach(async () => {
    fake = { github: [], minted: [], ended: [], prepared: [] };
    await harness.pool.query(`
      truncate table environment_saved_state_snapshot, environment, project, "user", organization cascade;
      insert into organization (id, name, slug) values ('${organizationId}', 'GitHub builds', 'github-builds');
      insert into "user" (id, email, name) values ('${userId}', 'owner@example.com', 'Owner');
      insert into project (id, organization_id, name, slug) values ('${projectId}', '${organizationId}', 'Cloud', 'cloud');
      insert into environment (id, project_id, organization_id, name, namespace, intent) values (
        '${environmentId}', '${projectId}', '${organizationId}', 'Production', 'production',
        '{"version":1,"environmentSlug":"production","services":[],"volumes":[]}');
      insert into organization_build_order (organization_id, build_order) values ('${organizationId}', 'github-only');
      insert into member (user_id, organization_id) values ('${userId}', '${organizationId}');
      insert into github_installation (user_id, installation_id, account_login, account_type) values ('${userId}', 7, 'owner', 'User');
      insert into github_repository_cache (user_id, installation_id, repository_id, name, full_name, default_branch, private, html_url, repo_updated_at)
        values ('${userId}', 7, 42, 'repo', 'owner/repo', 'main', true, 'https://github.com/owner/repo', now());
    `);
    await harness.db.insert(schema.environmentSavedStateSnapshot).values({
      id: savedId, organizationId, environmentId, actorId: userId, volumeDeletionAuthorizations: [],
      intent: { version: 1, environmentSlug: "production", services: [], volumes: [] },
    });
    await harness.db.insert(schema.environmentDeployment).values({
      id: deploymentId, organizationId, environmentId, savedStateSnapshotId: savedId,
      triggerOrigin: { origin: "manual", actorId: userId }, dispatchRequestedAt: new Date(),
      sourcePins: { [serviceId]: { commitSha: commit } },
    });
    await harness.db.insert(schema.environmentDeploymentSecret).values({ organizationId, environmentDeploymentId: deploymentId });
    await harness.db.insert(schema.serviceLineage).values({ id: serviceId, organizationId, projectId, canonicalName: "api", canonicalSlug: "api" });
    await harness.db.insert(schema.service).values({ id: serviceId, organizationId, projectId, environmentId, lineageId: serviceId, name: "api" });
    await harness.db.insert(schema.environmentNodeConfigSnapshot).values({
      organizationId, environmentDeploymentId: deploymentId, environmentId, nodeType: "service", nodeId: serviceId, nodeLineageId: serviceId,
      config: projectServiceDeploymentConfig({
        source: createGitServiceSource({ repository: "owner/repo", repositoryId: 42, access: { type: "github-installation", installationId: 7 } }),
        privateDns: "api", preDeployCommand: null, startCommand: null,
        healthcheck: createDefaultServiceHealthcheck(), restartPolicy: createDefaultServiceRestartPolicy(),
      }),
    });
  });

  type Services = Database | ReportingDatabase | OrganizationRuntime | AppConfig | GithubApi | GithubOidcKeys | InngestClient | SecretEncryption;
  const run = <A, E>(operation: Effect.Effect<A, E, Services>) => harness.runEffect(operation.pipe(
    Effect.provide(makeOrganizationRuntimeLayer(
      () => Effect.succeed({ kind: "ready", generation: "g", connections: [{ management: "ployz1:test", machine_id: machine.id }] }),
      noPairingChanges,
    ).pipe(Layer.provide(makePloyzLayer({ connect: async () => fakeClient(fake) })))),
    Effect.provide(AppConfig.layer),
    Effect.provideService(GithubApi, githubApi(fake)),
    Effect.provideService(GithubOidcKeys, { keys: Effect.succeed([jwk]) }),
    Effect.provideService(InngestClient, new Inngest({ id: "github-builds" })),
    Effect.provideService(SecretEncryption, encryption),
  ));
  // SAFETY: `run` supplies every service the deployment workflow's activities use.
  const runner = makeInngestEffectRunner(run) as typeof runInngestEffect;
  const imageBuildId = async () => (await harness.db.select().from(schema.environmentDeploymentImageBuild))[0]?.id ?? "";
  const row = async () => (await harness.db.select().from(schema.environmentDeploymentImageBuild))[0];
  const checkIn = async (token: string) => run(checkInGithubBuild(runnerRequest(token), await imageBuildId()));
  const rejection = async (token: string) => run(Effect.flip(checkInGithubBuild(runnerRequest(token), await imageBuildId())));
  const report = async (platforms: string[]) => run(recordGithubBuildSteps(runnerRequest(oidcToken(), {
    platforms,
    events: [
      { at: 1_000, event: { Build: { Stage: "Building" } } },
      { at: 2_000, event: { Build: { Step: { id: "s1", name: "RUN make", started: null, completed: null, cached: false, error: null } } } },
      { at: 3_000, event: { Build: { StepOutput: { step: "s1", stderr: false, text: "ok\n" } } } },
    ],
  }), await imageBuildId()));
  type MockedSteps = NonNullable<ConstructorParameters<typeof InngestTestEngine>[0]["steps"]>;
  const engine = (steps: MockedSteps = []) => new InngestTestEngine({
    function: createProcessEnvironmentDeployment(new Inngest({ id: "github-builds" }), runner),
    events: [{ name: "environment/deploy.requested", data: { environmentDeploymentId: deploymentId, environmentId } }],
    steps,
    transformCtx: (context) => {
      const ctx = mockCtx(context);
      // @inngest/test hands waitForEvent a lazy promise that inngest 4 then validates as an event,
      // so the run's completion is delivered by replacing the tool rather than mocking the step.
      const waitForEvent = async () => ({ name: "github/build-run.completed", data: { runId: githubRunId } });
      return { ...ctx, runId, step: { ...ctx.step, waitForEvent: asTestDouble<typeof ctx.step.waitForEvent>()(waitForEvent) } };
    },
  });

  /** Resumes a dispatched build: the dispatch already happened. */
  const runCompleted = (): MockedSteps => [
    { id: `start-github-build-${serviceId}`, handler: () => ({ kind: "dispatched", runId: githubRunId }) },
  ];

  it("dispatches the build workflow on the default branch with no secret inputs, and records the run", async () => {
    await engine().executeStep(`start-github-build-${serviceId}`);
    const dispatch = fake.github.find(({ operation }) => operation === "dispatch_workflow");
    expect(dispatch).toEqual({
      operation: "dispatch_workflow",
      url: "https://api.github.com/repos/owner/repo/actions/workflows/ployz-build.yml/dispatches",
      body: { ref: "main", return_run_details: true, inputs: { build: await imageBuildId(), cloud: "http://localhost:3000", ployz_version: expect.stringMatching(/^\d+\.\d+\.\d+/), runner: "ubuntu-latest" } },
    });
    expect(await row()).toMatchObject({ status: "building", builder: "github", githubRunId, githubWorkflowRef: workflowRef, checkedInAt: null });
  });

  it("rejects a check-in from another repository, workflow ref, run, or event, and a second use", async () => {
    await engine().executeStep(`start-github-build-${serviceId}`);
    expect(await rejection(oidcToken({ aud: "https://elsewhere.test" }))).toMatchObject({ _tag: "Unauthorized" });
    expect(await rejection(oidcToken({}, crypto.generateKeyPairSync("rsa", { modulusLength: 2048 }).privateKey))).toMatchObject({ _tag: "Unauthorized" });
    expect(await rejection(oidcToken({ repository_id: "43" }))).toMatchObject({ _tag: "Forbidden", message: "The token is for another repository." });
    expect(await rejection(oidcToken({ job_workflow_ref: "owner/repo/.github/workflows/ployz-build.yml@refs/heads/feature" })))
      .toMatchObject({ _tag: "Forbidden", message: "The token is for another workflow or branch." });
    expect(await rejection(oidcToken({ run_id: "9002" }))).toMatchObject({ _tag: "Forbidden", message: "The token is for another run." });
    expect(await rejection(oidcToken({ event_name: "push" }))).toMatchObject({ _tag: "Forbidden", message: "The run was not dispatched by Ployz." });
    expect(fake.minted).toEqual([]);

    const accepted = await checkIn(oidcToken());
    expect(accepted).toMatchObject({ grant: "ployzgrant1:secret", commit, fingerprint: expect.stringMatching(/^[0-9a-f]{64}$/) });
    expect(accepted).toHaveProperty("deployment.snapshots.0.config.privateDns", "api");
    expect(fake.minted).toEqual(["ployz-build/api"]);
    expect(await rejection(oidcToken())).toMatchObject({ _tag: "Conflict" });
    expect(fake.minted).toHaveLength(1);
  });

  it("writes the receipt from the digest the Machine received, shows the runner's steps, and deploys with it", async () => {
    await harness.db.update(schema.environmentDeployment).set({ status: "queued" }).where(eq(schema.environmentDeployment.id, deploymentId));
    // The runner, while Cloud waits for the run to complete: check in, build, report its steps.
    await engine().executeStep(`start-github-build-${serviceId}`);
    await checkIn(oidcToken());
    await report(["linux/amd64"]);
    const output = await engine(runCompleted()).execute();
    expect(output.error).toBeUndefined();
    const built = await row();
    expect(built).toMatchObject({ status: "built", machineId: machine.id, platforms: ["linux/amd64"] });
    const receipt = JSON.parse(encryption.decrypt(built?.encryptedReceipt ?? encryption.encrypt("null")));
    expect(receipt).toMatchObject({ machine_id: machine.id, fingerprint: built?.fingerprint, image: { reference: pushed, platforms: ["linux/amd64"] } });
    expect(fake.ended).toEqual(expect.arrayContaining(["f".repeat(64)]));
    expect(fake.prepared.at(-1)).toEqual({ api: receipt });

    const log = await harness.runEffect(loadDeploymentBuildLog({ organizationId, deploymentId, after: 0, limit: 50 }));
    expect(log.steps.filter((step) => step.image === "api").map((step) => step.name)).toEqual(["Building", "RUN make"]);
    expect(log.output.map((line) => line.text)).toEqual(["ok\n"]);
    expect(log.runs).toEqual([{ image: "api", url: "https://github.com/owner/repo/actions/runs/9001" }]);
    // A second report would duplicate output, so it is refused.
    expect(await run(Effect.flip(recordGithubBuildSteps(runnerRequest(oidcToken(), { platforms: [], events: [] }), built?.id ?? ""))))
      .toMatchObject({ _tag: "Conflict" });
  }, 30_000);

  it("fails the build when the run ends without pushing", async () => {
    await engine().executeStep(`start-github-build-${serviceId}`);
    await checkIn(oidcToken());
    await report([]);
    const output = await engine(runCompleted()).execute();
    expect(output.error).toEqual(expect.objectContaining({ message: "Image Build failed: api." }));
    expect(await row()).toMatchObject({ status: "failed", failureMessage: "GitHub: the run pushed no image." });
  });

  it("cancels the GitHub run and ends the grant when the attempt is cancelled", async () => {
    await engine().executeStep(`start-github-build-${serviceId}`);
    await checkIn(oidcToken());
    const cancelled = await new InngestTestEngine({
      function: createMarkCancelledRowBackedWorkflow(new Inngest({ id: "github-builds" }), runner),
      events: [{ name: "inngest/function.cancelled", data: { function_id: "process-environment-deployment", run_id: runId } }],
    }).execute();
    expect(cancelled.error).toBeUndefined();
    expect(fake.github).toContainEqual({ operation: "cancel_run", url: "https://api.github.com/repos/owner/repo/actions/runs/9001/cancel", body: undefined });
    expect(fake.ended).toEqual(["f".repeat(64)]);
    expect(await row()).toMatchObject({ status: "cancelled" });
  });
});
