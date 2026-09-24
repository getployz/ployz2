import { linkOptions } from "@tanstack/react-router";
import { Option, Schema } from "effect";
import { useEffect } from "react";
import { changeCollections } from "./collections";
import { useCollectionScope } from "./use-collection-scope";

const orgChangesEventSchema = Schema.Struct({ collections: Schema.Array(Schema.String) });
const decodeOrgChangesEvent = Schema.decodeUnknownOption(Schema.fromJsonString(orgChangesEventSchema));

export function buildOrgChangesUrl(organizationSlug: string) {
  const link = linkOptions({ to: "/api/org/changes", search: { organizationSlug } });
  return `${link.to}?${new URLSearchParams(link.search).toString()}`;
}

/** One change stream per Organization tab. Each named collection refetches only rows changed since its cursor. */
export function useOrganizationChanges(organizationSlug: string) {
  const { queryClient, sessionId, userId } = useCollectionScope();
  useEffect(() => {
    const scope = { queryClient, sessionId, userId };
    const refetch = (names: readonly string[]) => {
      for (const [name, get] of Object.entries(changeCollections)) {
        if (names.includes(name)) void get(organizationSlug, scope).utils.refetch();
      }
    };
    const source = new EventSource(buildOrgChangesUrl(organizationSlug));
    // Opening catches up on anything written before the stream started or while it was down.
    const handleOpen = () => refetch(Object.keys(changeCollections));
    const handleChanges = (event: MessageEvent<string>) => {
      const changes = decodeOrgChangesEvent(event.data);
      if (Option.isSome(changes)) refetch(changes.value.collections);
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
