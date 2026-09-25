/* oxlint-disable -- PROTOTYPE: throwaway fake-data UI on prototype/build-order-dashboard, never merged. */
// PROTOTYPE (prototype/build-order-dashboard): per-Service preferred builder, fake data.
import { Field, FieldLabel } from "#/components/ui/field";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "#/components/ui/select";
import { update, useBuildOrderState } from "./store";

export function ServiceBuildOnField({ serviceId }: { serviceId: string }) {
  const { preferredBuilder, servers } = useBuildOrderState();
  const builders = [
    { id: "github", label: "GitHub Actions" },
    ...servers.filter((server) => server.builds).map((server) => ({ id: server.id, label: server.name })),
  ];
  const value = preferredBuilder[serviceId] ?? "auto";
  const label = value === "auto" ? "Auto" : builders.find((builder) => builder.id === value)?.label ?? value;
  return (
    <Field>
      <FieldLabel htmlFor="service-preferred-builder">Preferred builder</FieldLabel>
      <Select
        value={value}
        onValueChange={(next) =>
          update((s) => ({ ...s, preferredBuilder: { ...s.preferredBuilder, [serviceId]: next === "auto" ? undefined : String(next) } }))
        }
      >
        <SelectTrigger id="service-preferred-builder" className="w-64">
          <SelectValue>{label}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectItem value="auto" label="Auto">Auto</SelectItem>
          </SelectGroup>
          <SelectSeparator />
          <SelectGroup>
            {builders.map((builder) => (
              <SelectItem key={builder.id} value={builder.id} label={builder.label}>{builder.label}</SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
    </Field>
  );
}
