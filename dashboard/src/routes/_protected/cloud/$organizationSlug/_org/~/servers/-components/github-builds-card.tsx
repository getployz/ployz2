import { useState } from "react";
import { CheckIcon } from "lucide-react";
import { Button } from "#/components/ui/button";
import { Card, CardAction, CardDescription, CardHeader, CardTitle } from "#/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "#/components/ui/dialog";
import { githubBuildRepositoryKey, githubBuildWorkflowUrl, type GithubBuildRepository } from "#/modules/github/github-build-workflow";
import { useGithubBuildRepositories } from "#/modules/github/github.queries";

/** GitHub Actions setup for the repositories this organization's Services build from. Hidden when there are none. */
export function GithubBuildsCard({ organizationSlug }: { organizationSlug: string }) {
  // ponytail: "waiting for the commit" lives in this tab only; persist it if teammates need to see it.
  const [opened, setOpened] = useState<ReadonlySet<string>>(new Set());
  const { data: repositories, error } = useGithubBuildRepositories(organizationSlug, opened);
  const waiting = (repository: GithubBuildRepository) =>
    repository.readiness === "setup_needed" && opened.has(githubBuildRepositoryKey(repository));

  if (error && !repositories) {
    return (
      <Card size="sm">
        <CardHeader>
          <CardTitle>GitHub Actions</CardTitle>
          <CardDescription>Could not check the repositories on GitHub.</CardDescription>
        </CardHeader>
      </Card>
    );
  }
  if (!repositories?.length) return null;
  const ready = repositories.filter((repository) => repository.readiness === "ready").length;

  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>GitHub Actions</CardTitle>
        <CardDescription>
          {ready} of {repositories.length} repositories set up
        </CardDescription>
        <CardAction>
          <Dialog>
            <DialogTrigger render={<Button size="sm" variant="outline" />}>Manage</DialogTrigger>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>GitHub Actions</DialogTitle>
                <DialogDescription>
                  Each repository needs one small workflow file. It only runs when Ployz starts a build, and build secrets are sent to the runner.
                </DialogDescription>
              </DialogHeader>
              <div className="flex flex-col divide-y rounded-lg border">
                {repositories.map((repository) => (
                  <div key={githubBuildRepositoryKey(repository)} className="flex h-11 items-center justify-between gap-3 px-3">
                    <span className="truncate font-mono text-sm">{repository.fullName}</span>
                    {repository.readiness === "ready" ? (
                      <CheckIcon className="size-4 text-success" aria-label="Set up" />
                    ) : repository.readiness === "no_permission" ? (
                      <span className="text-muted-foreground text-sm">Installation lacks permission</span>
                    ) : waiting(repository) ? (
                      <span className="text-muted-foreground text-sm">Waiting for commit</span>
                    ) : repository.defaultBranch === null ? null : (
                      <Button
                        size="xs"
                        variant="ink"
                        nativeButton={false}
                        render={
                          <a
                            href={githubBuildWorkflowUrl({ fullName: repository.fullName, defaultBranch: repository.defaultBranch })}
                            target="_blank"
                            rel="noreferrer"
                          />
                        }
                        onClick={() => setOpened((current) => new Set(current).add(githubBuildRepositoryKey(repository)))}
                      >
                        Add workflow ↗
                      </Button>
                    )}
                  </div>
                ))}
              </div>
            </DialogContent>
          </Dialog>
        </CardAction>
      </CardHeader>
    </Card>
  );
}
