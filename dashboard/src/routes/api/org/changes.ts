import { createFileRoute } from "@tanstack/react-router";
import { Effect, Option, Schema } from "effect";
import { collectionsOf } from "#/collections/change-sources";
import { readChangeWindow } from "#/collections/changes.server";
import { organizationSlugSchema } from "#/modules/organization/tables";
import { authorizeRuntimeOrganization } from "#/modules/runtime/authorize-runtime-organization.server";
import { handleOrgChangesRequest, orgChangesValidationError } from "#/routes/api/org/-changes.handler";
import { publicErrorResponse } from "#/server/public-error";
import { runAppEffect } from "#/server/run.server";

const OrgChangesSearch = Schema.Struct({ organizationSlug: organizationSlugSchema });
const decodeOrgChangesSearch = Schema.decodeUnknownOption(OrgChangesSearch);

export const Route = createFileRoute("/api/org/changes")({
  validateSearch: Schema.toStandardSchemaV1(OrgChangesSearch),
  server: {
    handlers: {
      GET: async ({ request }) => {
        const search = decodeOrgChangesSearch(Object.fromEntries(new URL(request.url).searchParams));
        if (Option.isNone(search)) return publicErrorResponse(orgChangesValidationError());
        return handleOrgChangesRequest(request, search.value.organizationSlug, {
          authorize: ({ organizationSlug }) =>
            runAppEffect(authorizeRuntimeOrganization({ headers: request.headers, organizationSlug }), { signal: request.signal }),
          readChanges: (input) =>
            runAppEffect(readChangeWindow(input).pipe(
              Effect.map((window) => ({ cursor: window.cursor, expired: window.expired, collections: collectionsOf(window.sourceTables) })),
            ), { signal: request.signal }),
        });
      },
    },
  },
});
