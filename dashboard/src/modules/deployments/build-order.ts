import { Schema } from "effect";
import { OrganizationSlug } from "#/modules/environment-design/workspace-schemas";

export const BUILD_ORDERS = ["servers-only", "github-then-servers", "github-only"] as const;
export type BuildOrder = (typeof BUILD_ORDERS)[number];

export const BUILD_ORDER_LABELS = {
  "servers-only": "Your servers only",
  "github-then-servers": "GitHub, then your servers",
  "github-only": "GitHub only",
} satisfies Record<BuildOrder, string>;

/** The Organization's Build Order as the Org Store holds it: one row, keyed by the Organization. */
export type BuildOrderRow = { id: string; buildOrder: BuildOrder };

export const DEFAULT_BUILD_ORDER: BuildOrder = "servers-only";

export const buildOrderEditSchema = Schema.Struct({
  organizationSlug: OrganizationSlug,
  buildOrder: Schema.Literals(BUILD_ORDERS),
});
