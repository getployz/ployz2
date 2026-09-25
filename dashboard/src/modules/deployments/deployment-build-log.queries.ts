import { queryOptions, skipToken, useQuery } from "@tanstack/react-query";
import { listDeploymentBuildLogServerFn } from "./deployment.functions";

type BuildLogPage = Awaited<ReturnType<typeof listDeploymentBuildLogServerFn>>;
export type BuildStepRow = BuildLogPage["steps"][number];
export type BuildOutputRow = BuildLogPage["output"][number];

type BuildLog = { steps: BuildStepRow[]; output: BuildOutputRow[]; serverChoices: BuildLogPage["serverChoices"]; finished: boolean };

/** Polls the step tree while the attempt runs, resuming output from the last row already held. */
export function deploymentBuildLogQueryOptions(organizationSlug: string, deploymentId: string) {
  const queryKey = ["deployment-build-log", organizationSlug, deploymentId];
  return queryOptions<BuildLog>({
    queryKey,
    // Each mount resumes from the last held row; a running build also polls.
    staleTime: 0,
    refetchInterval: (query) => query.state.data?.finished ? false : 2_000,
    queryFn: async ({ signal, client }) => {
      const previous = client.getQueryData<BuildLog>(queryKey);
      let steps: BuildStepRow[] = [];
      let serverChoices: BuildLogPage["serverChoices"] = [];
      const output: BuildOutputRow[] = [...previous?.output ?? []];
      let finished = false;
      const last = output.at(-1);
      let afterSequence: string | null | undefined = last ? String(last.id) : undefined;
      while (afterSequence !== null) {
        const page = await listDeploymentBuildLogServerFn({ data: { organizationSlug, deploymentId, afterSequence, limit: 100 }, signal });
        steps = page.steps;
        serverChoices = page.serverChoices;
        output.push(...page.output);
        finished = page.finished;
        afterSequence = page.nextSequence;
      }
      return { steps, output, serverChoices, finished };
    },
  });
}

/** Reads nothing without an attempt. */
export function useBuildLog(organizationSlug: string, deploymentId: string | null) {
  return useQuery(deploymentId === null
    ? { queryKey: ["deployment-build-log", organizationSlug, null], queryFn: skipToken }
    : deploymentBuildLogQueryOptions(organizationSlug, deploymentId));
}
