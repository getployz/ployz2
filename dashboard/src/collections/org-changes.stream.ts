import { linkOptions } from "@tanstack/react-router";
import { Option, Schema } from "effect";
import { useEffect } from "react";
import { changeCollections } from "./collections";
import type { CollectionScope } from "./scope";
import { useCollectionScope } from "./use-collection-scope";
import { organizationKeys } from "#/modules/environment-design/workspace.queries";

const orgChangesEventSchema = Schema.Struct({ collections: Schema.Array(Schema.String) });
const decodeOrgChangesEvent = Schema.decodeUnknownOption(Schema.fromJsonString(orgChangesEventSchema));

export function buildOrgChangesUrl(organizationSlug: string) {
  const link = linkOptions({ to: "/api/org/changes", search: { organizationSlug } });
  return `${link.to}?${new URLSearchParams(link.search).toString()}`;
}

/** Refetches each named collection since its cursor; `organization` re-reads the organization state (its name). */
export function applyOrganizationChanges(names: readonly string[], organizationSlug: string, scope: CollectionScope) {
  for (const [name, get] of Object.entries(changeCollections)) {
    if (names.includes(name)) void get(organizationSlug, scope).utils.refetch();
  }
  if (names.includes("organization")) void scope.queryClient.invalidateQueries({ queryKey: organizationKeys.all });
}

/** One change stream per Organization tab. Each named collection refetches only rows changed since its cursor. */
export function useOrganizationChanges(organizationSlug: string) {
  const { queryClient, sessionId, userId } = useCollectionScope();
  useEffect(() => {
    const scope = { queryClient, sessionId, userId };
    const source = new EventSource(buildOrgChangesUrl(organizationSlug));
    // Opening catches up on anything written before the stream started or while it was down.
    const handleOpen = () => applyOrganizationChanges([...Object.keys(changeCollections), "organization"], organizationSlug, scope);
    const handleChanges = (event: MessageEvent<string>) => {
      const changes = decodeOrgChangesEvent(event.data);
      if (Option.isSome(changes)) applyOrganizationChanges(changes.value.collections, organizationSlug, scope);
    };
    source.addEventListener("open", handleOpen);
    source.addEventListener("changes", handleChanges);
    return () => {
      source.removeEventListener("open", handleOpen);
      source.removeEventListener("changes", handleChanges);
      source.close();
    };
  }, [organizationSlug, queryClient, sessionId, userId]);
}
