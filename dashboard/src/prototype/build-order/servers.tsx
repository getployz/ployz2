/* oxlint-disable -- PROTOTYPE: throwaway fake-data UI on prototype/build-order-dashboard, never merged. */
// PROTOTYPE (prototype/build-order-dashboard): Servers page build controls and Build Order, fake data.
import { CheckIcon, MoreHorizontalIcon } from "lucide-react";
import { Button } from "#/components/ui/button";
import { Card, CardAction, CardContent, CardHeader, CardTitle } from "#/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "#/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "#/components/ui/dropdown-menu";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "#/components/ui/select";
import { Switch } from "#/components/ui/switch";
import {
  automaticConcurrency,
  BUILD_ORDER_LABELS,
  type BuildOrder,
  type FakeServer,
  update,
  useBuildOrderState,
} from "./store";

const ORDERS = Object.keys(BUILD_ORDER_LABELS) as BuildOrder[];

export function FakeServerRows() {
  const { servers } = useBuildOrderState();
  return servers.map((server) => <FakeServerRow key={server.id} server={server} />);
}

function FakeServerRow({ server }: { server: FakeServer }) {
  const building = server.builds && server.buildingNow.length > 0;
  const patch = (next: Partial<FakeServer>) =>
    update((s) => ({ ...s, servers: s.servers.map((row) => (row.id === server.id ? { ...row, ...next } : row)) }));

  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          {server.name}
          {building ? (
            <span className="relative flex size-2" title={`Building ${server.buildingNow.join(", ")}`}>
              <span className="absolute inline-flex size-full animate-ping rounded-full bg-info opacity-75" />
              <span className="relative inline-flex size-2 rounded-full bg-info" />
            </span>
          ) : null}
        </CardTitle>
        <CardAction>
          <div className="flex items-center gap-3">
            <label className="flex items-center gap-2 text-muted-foreground text-sm">
              Builds
              <Switch size="sm" checked={server.builds} onCheckedChange={(builds) => patch({ builds })} />
            </label>
            <DropdownMenu>
              <DropdownMenuTrigger render={<Button variant="ghost" size="icon" aria-label={`Actions for ${server.name}`} />}>
                <MoreHorizontalIcon />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuGroup>
                  <DropdownMenuLabel>Builds at once</DropdownMenuLabel>
                  <DropdownMenuRadioGroup
                    value={server.concurrency === null ? "auto" : String(server.concurrency)}
                    onValueChange={(value) => patch({ concurrency: value === "auto" ? null : Number(value) })}
                  >
                    <DropdownMenuRadioItem value="auto">Automatic ({automaticConcurrency(server)})</DropdownMenuRadioItem>
                    {[1, 2, 4, 8].map((n) => (
                      <DropdownMenuRadioItem key={n} value={String(n)}>{n}</DropdownMenuRadioItem>
                    ))}
                  </DropdownMenuRadioGroup>
                </DropdownMenuGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </CardAction>
      </CardHeader>
    </Card>
  );
}

export function BuildOrderSelect({
  value,
  onChange,
  defaultLabel,
  id,
}: {
  value: BuildOrder | "default";
  onChange: (next: BuildOrder | "default") => void;
  defaultLabel?: string;
  id: string;
}) {
  const label = value === "default" ? `Default · ${defaultLabel}` : BUILD_ORDER_LABELS[value];
  return (
    <Select value={value} onValueChange={(next) => onChange(next as BuildOrder | "default")}>
      <SelectTrigger id={id} className="w-64">
        <SelectValue>{label}</SelectValue>
      </SelectTrigger>
      <SelectContent>
        <SelectGroup>
          {defaultLabel ? (
            <SelectItem value="default" label={`Default · ${defaultLabel}`}>Default · {defaultLabel}</SelectItem>
          ) : null}
          {ORDERS.map((order) => (
            <SelectItem key={order} value={order} label={BUILD_ORDER_LABELS[order]}>{BUILD_ORDER_LABELS[order]}</SelectItem>
          ))}
        </SelectGroup>
      </SelectContent>
    </Select>
  );
}

export function BuildOrderSection() {
  const { buildOrder, repos } = useBuildOrderState();
  const ready = repos.filter((repo) => repo.readiness === "ready").length;

  return (
    <Card>
      <CardHeader>
        <CardTitle>Builds</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col divide-y">
        <div className="flex items-center justify-between gap-4 pb-3">
          <label htmlFor="build-order" className="text-sm">Build on</label>
          <BuildOrderSelect
            id="build-order"
            value={buildOrder}
            onChange={(next) => next !== "default" && update((s) => ({ ...s, buildOrder: next }))}
          />
        </div>
        {buildOrder === "servers-only" ? null : (
          <div className="flex items-center justify-between gap-4 pt-3">
            <span className="text-sm">GitHub Actions</span>
            <span className="flex items-center gap-3 text-muted-foreground text-sm">
              {ready} of {repos.length} repositories set up
              <GithubReposDialog />
            </span>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

const WORKFLOW = `name: Ployz build
on:
  workflow_dispatch:
    inputs:
      build: { required: true, type: string }
      runner: { required: false, type: string, default: ubuntu-latest }
permissions:
  contents: read
  id-token: write
jobs:
  build:
    runs-on: \${{ inputs.runner }}
    steps:
      - uses: ployz/build@v1
        with:
          build: \${{ inputs.build }}
`;

/** GitHub's new-file page with the workflow filled in; the user commits it (or opens a PR) themselves. */
function workflowFileUrl(fullName: string) {
  const params = new URLSearchParams({ filename: ".github/workflows/ployz-build.yml", value: WORKFLOW });
  return `https://github.com/${fullName}/new/main?${params.toString()}`;
}

function GithubReposDialog() {
  const { repos } = useBuildOrderState();
  const setReadiness = (fullName: string, readiness: "ready" | "pr-open") =>
    update((s) => ({ ...s, repos: s.repos.map((repo) => (repo.fullName === fullName ? { ...repo, readiness } : repo)) }));
  return (
    <Dialog>
      <DialogTrigger render={<Button size="sm" variant="outline" />}>Manage</DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>GitHub Actions</DialogTitle>
          <DialogDescription>Each repository needs one small workflow file. It only runs when Ployz starts a build, and build secrets are sent to the runner.</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col divide-y rounded-lg border">
          {repos.map((repo) => (
            <div key={repo.fullName} className="flex h-11 items-center justify-between gap-3 px-3">
              <span className="font-mono text-sm">{repo.fullName}</span>
              {repo.readiness === "ready" ? (
                <CheckIcon className="size-4 text-success" aria-label="Set up" />
              ) : repo.readiness === "pr-open" ? (
                <Button size="xs" variant="ghost" onClick={() => setReadiness(repo.fullName, "ready")}>
                  Waiting for commit · simulate
                </Button>
              ) : (
                <Button
                  size="xs"
                  variant="ink"
                  nativeButton={false}
                  render={<a href={workflowFileUrl(repo.fullName)} target="_blank" rel="noreferrer" />}
                  onClick={() => setReadiness(repo.fullName, "pr-open")}
                >
                  Add workflow ↗
                </Button>
              )}
            </div>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}
