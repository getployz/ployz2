/* oxlint-disable -- PROTOTYPE: throwaway fake-data UI on prototype/build-order-dashboard, never merged. */
// PROTOTYPE (prototype/build-order-dashboard): Servers page build controls and Build Order, fake data.
import { GitPullRequestIcon, HammerIcon } from "lucide-react";
import { Badge } from "#/components/ui/badge";
import { Button } from "#/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "#/components/ui/card";
import { Field, FieldDescription, FieldLabel } from "#/components/ui/field";
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
  BUILD_ORDER_DESCRIPTIONS,
  BUILD_ORDER_LABELS,
  type BuildOrder,
  type FakeServer,
  update,
  useBuildOrderState,
} from "./store";

const ORDERS = Object.keys(BUILD_ORDER_LABELS) as BuildOrder[];

export function BuildOrderOption({ order }: { order: BuildOrder }) {
  return (
    <span className="grid gap-1 whitespace-normal">
      <span>{BUILD_ORDER_LABELS[order]}</span>
      <span className="text-muted-foreground">{BUILD_ORDER_DESCRIPTIONS[order]}</span>
    </span>
  );
}

export function FakeServerRows() {
  const { servers } = useBuildOrderState();
  return servers.map((server) => <FakeServerRow key={server.id} server={server} />);
}

function FakeServerRow({ server }: { server: FakeServer }) {
  const auto = automaticConcurrency(server);
  const description = [
    server.runsServices ? "runs Services" : "no Services",
    `${server.ramGb} GB RAM`,
    server.builds && server.cacheFor.length > 0 ? `build cache: ${server.cacheFor.join(", ")}` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  function patch(next: Partial<FakeServer>) {
    update((s) => ({
      ...s,
      servers: s.servers.map((row) => (row.id === server.id ? { ...row, ...next } : row)),
    }));
  }

  return (
    <Card size="sm">
      <CardHeader className="flex flex-col gap-2 sm:grid">
        <CardTitle className="flex items-center gap-2">
          {server.name}
          {server.builds && server.buildingNow.length > 0 ? (
            <Badge variant="info">
              <HammerIcon data-icon="inline-start" />
              building {server.buildingNow.join(", ")}
            </Badge>
          ) : null}
        </CardTitle>
        <CardDescription>{description}</CardDescription>
        <CardAction>
          <div className="flex items-center gap-4">
            <label className="flex items-center gap-2 text-sm">
              <Switch checked={server.builds} onCheckedChange={(builds) => patch({ builds })} />
              Builds
            </label>
            <Select
              value={server.concurrency === null ? "auto" : String(server.concurrency)}
              disabled={!server.builds}
              onValueChange={(value) =>
                patch({ concurrency: value === "auto" ? null : Number(value) })
              }
            >
              <SelectTrigger aria-label={`${server.name} concurrent builds`} className="w-44">
                <SelectValue>
                  {server.concurrency === null
                    ? `Automatic · ${auto} at once`
                    : `${server.concurrency} at once`}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectItem value="auto" label={`Automatic · ${auto} at once`}>
                    Automatic · {auto} at once
                  </SelectItem>
                  {[1, 2, 3, 4, 6, 8].map((n) => (
                    <SelectItem key={n} value={String(n)} label={`${n} at once`}>
                      {n} at once
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
          </div>
        </CardAction>
      </CardHeader>
    </Card>
  );
}

export function BuildOrderSection() {
  const { buildOrder, repos, servers } = useBuildOrderState();
  const building = servers.filter((server) => server.builds);
  const usesGithub = buildOrder !== "servers-only";

  return (
    <Card>
      <CardHeader>
        <CardTitle>Build order</CardTitle>
        <CardDescription>
          Where your images build. A build moves to the next builder only if it
          hasn't started in time. A build that has started never moves.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-6">
        <Field>
          <FieldLabel htmlFor="build-order">Build on</FieldLabel>
          <Select
            value={buildOrder}
            onValueChange={(next) => update((s) => ({ ...s, buildOrder: next as BuildOrder }))}
          >
            <SelectTrigger id="build-order" className="w-full data-[size=default]:h-auto">
              <SelectValue>
                <BuildOrderOption order={buildOrder} />
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                {ORDERS.map((order) => (
                  <SelectItem key={order} value={order} label={BUILD_ORDER_LABELS[order]}>
                    <BuildOrderOption order={order} />
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
          <FieldDescription>
            Your servers: {building.map((server) => server.name).join(", ") || "none build"}. Ployz picks
            the one with the Service's build cache, then the one with the most free slots.
            {usesGithub ? " Build secrets are sent to the GitHub runner for each build." : null}
          </FieldDescription>
        </Field>

        {usesGithub ? (
          <Field>
            <FieldLabel>GitHub Actions</FieldLabel>
            <FieldDescription>
              Each repository builds its own Services with a small workflow that only
              runs when Ployz starts it.
            </FieldDescription>
            <div className="flex flex-col divide-y rounded-lg border">
              {repos.map((repo) => (
                <div key={repo.fullName} className="flex items-center gap-3 px-3 py-2">
                  <span className="font-mono text-xs">{repo.fullName}</span>
                  <span className="text-muted-foreground text-xs">{repo.services.join(", ")}</span>
                  <span className="ml-auto flex items-center gap-2">
                    {repo.readiness === "ready" ? (
                      <Badge variant="success">Ready</Badge>
                    ) : repo.readiness === "pr-open" ? (
                      <>
                        <Badge variant="warning">Setup PR open</Badge>
                        <Button
                          size="xs"
                          variant="outline"
                          onClick={() => setReadiness(repo.fullName, "ready")}
                        >
                          Simulate merge
                        </Button>
                      </>
                    ) : (
                      <>
                        <Badge variant="outline">Skipped until set up</Badge>
                        <Button
                          size="xs"
                          variant="ink"
                          onClick={() => setReadiness(repo.fullName, "pr-open")}
                        >
                          <GitPullRequestIcon data-icon="inline-start" />
                          Open setup PR
                        </Button>
                      </>
                    )}
                  </span>
                </div>
              ))}
            </div>
          </Field>
        ) : null}
      </CardContent>
    </Card>
  );
}

function setReadiness(fullName: string, readiness: "ready" | "pr-open") {
  update((s) => ({
    ...s,
    repos: s.repos.map((repo) => (repo.fullName === fullName ? { ...repo, readiness } : repo)),
  }));
}
