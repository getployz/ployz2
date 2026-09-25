import { useState } from "react";
import { eq, useLiveSuspenseQuery } from "@tanstack/react-db";
import type { ServiceConfig } from "@ployz/sdk/config";
import { useCollectionScope } from "#/collections/use-collection-scope";
import { getRawServicesCollection } from "#/collections/collections";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "#/components/ui/collapsible";
import { useDeploymentServiceVariables } from "#/modules/deployments/deployment-variables.queries";
import { DeployedVariableValue } from "./DeployedVariableValue";

/** The attempt's variables for one service, resolved as deployed; read when opened. */
export function DeployedVariables({ organizationSlug, deploymentId, environmentId, serviceId, env }: {
  organizationSlug: string;
  deploymentId: string;
  environmentId: string;
  serviceId: string;
  env: ServiceConfig["env"];
}) {
  const [open, setOpen] = useState(false);
  const values = useDeploymentServiceVariables(organizationSlug, deploymentId, serviceId, open);
  const services = getRawServicesCollection(organizationSlug, useCollectionScope());
  const { data: named } = useLiveSuspenseQuery({
    queryKey: ["deployed-variables-services", services.id, environmentId],
    query: (q) => q.from({ service: services }).where(({ service }) => eq(service.environmentId, environmentId))
      .select(({ service }) => ({ id: service.id, lineageId: service.lineageId, name: service.name })),
  });
  const variables = Object.entries(env).sort(([a], [b]) => a.localeCompare(b));
  // The services a template reads from; a deleted one resolved to "".
  const sources = (value: ServiceConfig["env"][string]) => value.kind === "literal" && value.parts
    ? [...new Set(value.parts.flatMap((part) => part.kind !== "ref" ? []
      : [named.find((service) => part.owner.scope === "self" ? service.id === serviceId : service.lineageId === part.owner.lineageId)?.name ?? "a deleted service"]))]
    : [];

  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <CollapsibleTrigger className="cursor-pointer font-medium">
        {variables.length} {variables.length === 1 ? "variable" : "variables"} (as deployed)
      </CollapsibleTrigger>
      <CollapsibleContent>
        {values.isError ? <p className="mt-2 text-destructive">{values.error.message}</p> : (
          <dl className="mt-2 grid grid-cols-[auto_1fr] items-center gap-x-4 gap-y-1 font-mono text-xs">
            {variables.flatMap(([key, value]) => [
              <dt key={`${key}:dt`}>{key}</dt>,
              <dd key={`${key}:dd`} className="min-w-0">
                <DeployedVariableValue name={key} value={values.data === undefined ? undefined : values.data[key] ?? null} from={sources(value)} />
              </dd>,
            ])}
          </dl>
        )}
      </CollapsibleContent>
    </Collapsible>
  );
}
