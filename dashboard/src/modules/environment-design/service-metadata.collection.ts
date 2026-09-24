import { toast } from "sonner";
import { createOptimisticAction } from "@tanstack/react-db";
import { cachedByCollectionScope } from "#/collections/scope";
import { useCollectionScope } from "#/collections/use-collection-scope";
import { getRawServicesCollection } from "#/collections/collections";
import { editServiceMetadataServerFn } from "./service-metadata.functions";
import type { ServiceMetadataEdit } from "./service-metadata";

const getServiceMetadataEditor = cachedByCollectionScope((organizationSlug, scope) => {
  const identities = getRawServicesCollection(organizationSlug, scope);
  return createOptimisticAction<Omit<ServiceMetadataEdit, "organizationSlug">>({
    onMutate: ({ serviceId, edit }) => identities.update(serviceId, draft => {
      if (edit.kind === "rename") draft.name = edit.name;
      else Object.assign(draft.policy, structuredClone(edit.policy));
    }),
    mutationFn: async (input) => {
      try {
        const result = await editServiceMetadataServerFn({ data: { ...input, organizationSlug } });
        await identities.writeCommitted(result.data);
      } catch (error) {
        toast.error(error instanceof Error ? error.message : "Could not save this setting.");
        throw error;
      }
    },
  });
});

export function useServiceMetadataEditor(organizationSlug: string) {
  const edit = getServiceMetadataEditor(organizationSlug, useCollectionScope());
  return (input: Parameters<typeof edit>[0]) => {
    const transaction = edit(input);
    // The failure is already toasted; observing it keeps fire-and-forget callers free of unhandled rejections.
    transaction.isPersisted.promise.catch(() => {});
    return transaction;
  };
}
