import { Schema } from "effect";

export const SERVICE_PAGES = [
  { id: "settings", label: "Configuration" },
  { id: "variables", label: "Environment variables" },
  { id: "deployments", label: "Deployments" },
] as const;

export type ServicePage = (typeof SERVICE_PAGES)[number]["id"];

export const servicePageSchema = Schema.Literals(SERVICE_PAGES.map((page) => page.id));

export const serviceSearchSchema = Schema.Struct({
  tab: Schema.optional(servicePageSchema),
});
