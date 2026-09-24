import { linkOptions } from "@tanstack/react-router";
import { Option, Schema } from "effect";
import { useEffect } from "react";
import { changeCollections } from "./collections";
import { changeNameSchema, type ChangeName } from "./read.contract";
import type { CollectionScope } from "./scope";
import { useCollectionScope } from "./use-collection-scope";
import { organizationKeys } from "#/modules/environment-design/workspace.queries";

const orgChangesEventSchema = Schema.Struct({ collections: Schema.Array(changeNameSchema) });
const decodeOrgChangesEvent = Schema.decodeUnknownOption(Schema.fromJsonString(orgChangesEventSchema));

export function buildOrgChangesUrl(organizationSlug: string) {
  const link = linkOptions({ to: "/api/org/changes", search: { organizationSlug } });
  return `${link.to}?${new URLSearchParams(link.search).toString()}`;
}

/** Refetches each named collection since its cursor; `organization` re-reads the organization state (its name). */
export function applyOrganizationChanges(names: readonly ChangeName[], organizationSlug: string, scope: CollectionScope) {
  for (const name of names) {
    if (name === "organization") void scope.queryClient.invalidateQueries({ queryKey: organizationKeys.all });
    else void changeCollections.get(name)?.(organizationSlug, scope).utils.refetch();
  }
}

/** One change stream per Organization tab. Each named collection refetches only rows changed since its cursor. */
export function useOrganizationChanges(organizationSlug: string) {
  const { queryClient, sessionId, userId } = useCollectionScope();
  useEffect(
    () => watchOrganizationChanges(organizationSlug, { queryClient, sessionId, userId }),
    [organizationSlug, queryClient, sessionId, userId],
  );
}

export function watchOrganizationChanges(organizationSlug: string, scope: CollectionScope) {
  const source = new EventSource(buildOrgChangesUrl(organizationSlug));
  // `reset` means retention passed the resume point; each collection's own cursor decides whether it reads in full.
  const refetchAll = () => applyOrganizationChanges([...changeCollections.keys(), "organization"], organizationSlug, scope);
  const handleChanges = (event: MessageEvent<string>) => {
    const changes = decodeOrgChangesEvent(event.data);
    if (Option.isSome(changes)) applyOrganizationChanges(changes.value.collections, organizationSlug, scope);
  };
  // A first connect has no Last-Event-ID, so opening refetches to cover writes between the preload and the first cursor.
  source.addEventListener("open", refetchAll);
  source.addEventListener("reset", refetchAll);
  source.addEventListener("changes", handleChanges);
  return () => source.close();
}
