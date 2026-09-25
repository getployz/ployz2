/* oxlint-disable -- PROTOTYPE: throwaway fake-data UI on prototype/build-order-dashboard, never merged. */
// PROTOTYPE (prototype/build-order-dashboard): per-Service Build Order override, fake data.
import { Link, useParams } from "@tanstack/react-router";
import { Field, FieldDescription, FieldLabel } from "#/components/ui/field";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "#/components/ui/select";
import { BuildOrderOption } from "./servers";
import { BUILD_ORDER_LABELS, type BuildOrder, update, useBuildOrderState } from "./store";

const ORDERS = Object.keys(BUILD_ORDER_LABELS) as BuildOrder[];

export function ServiceBuildOnField({ serviceId }: { serviceId: string }) {
  const { organizationSlug } = useParams({ strict: false });
  const { buildOrder, serviceOverrides } = useBuildOrderState();
  const override = serviceOverrides[serviceId];
  const value = override ?? "default";

  return (
    <Field>
      <FieldLabel htmlFor="service-build-on">Build on</FieldLabel>
      <FieldDescription>
        Takes effect on the next build. It isn't a staged change.
      </FieldDescription>
      <Select
        value={value}
        onValueChange={(next) =>
          update((s) => ({
            ...s,
            serviceOverrides: {
              ...s.serviceOverrides,
              [serviceId]: next === "default" ? undefined : (next as BuildOrder),
            },
          }))
        }
      >
        <SelectTrigger id="service-build-on" className="w-full data-[size=default]:h-auto">
          <SelectValue>
            {override ? (
              <BuildOrderOption order={override} />
            ) : (
              <span className="grid gap-1 whitespace-normal">
                <span>Build order (default)</span>
                <span className="text-muted-foreground">{BUILD_ORDER_LABELS[buildOrder]}</span>
              </span>
            )}
          </SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectItem value="default" label="Build order (default)">
              <span className="grid gap-1 whitespace-normal">
                <span>Build order (default)</span>
                <span className="text-muted-foreground">{BUILD_ORDER_LABELS[buildOrder]}</span>
              </span>
            </SelectItem>
          </SelectGroup>
          <SelectSeparator />
          <SelectGroup>
            {ORDERS.map((order) => (
              <SelectItem key={order} value={order} label={BUILD_ORDER_LABELS[order]}>
                <BuildOrderOption order={order} />
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
      {organizationSlug ? (
        <FieldDescription>
          Change the default under{" "}
          <Link to="/cloud/$organizationSlug/~/servers" params={{ organizationSlug }}>
            Servers ▸ Build order
          </Link>
          .
        </FieldDescription>
      ) : null}
    </Field>
  );
}
