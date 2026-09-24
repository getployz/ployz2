import { toast } from "sonner";
import { createOptimisticAction } from "@tanstack/react-db";
import { getEnvironmentsCollection } from "#/collections/collections";
import { cachedByCollectionScope, type CollectionScope } from "#/collections/scope";
import { useCollectionScope } from "#/collections/use-collection-scope";
import type { environment as schemaEnvironment } from "#/modules/project/tables";
import type { SavedEnvironmentIntent } from "./saved-intent";

type SavedEnvironment = typeof schemaEnvironment.$inferSelect;

export type EnvironmentDocumentEdit = {
  environmentId: string;
  /** Applied to the in-memory document immediately; rolled back if saving fails. */
  apply: (intent: SavedEnvironmentIntent) => void;
  /** Saves against `revision` and returns the saved environment. */
  save: (revision: string) => Promise<{ data: SavedEnvironment }>;
  /** Toasted when saving fails and the edit rolls back. */
  failureMessage: string;
  /** Runs after the saved environment is committed, before the edit counts as persisted. */
  afterSave?: () => Promise<void>;
};

type QueuedEdit = EnvironmentDocumentEdit & { revision: string };

/**
 * The one way to change an environment document: apply now, save in the background,
 * roll back and toast on failure. Callers do not wait.
 *
 * The server accepts only the current revision, so saves to one environment run in
 * order and each uses the revision the previous save returned. Optimistic snapshots
 * keep the revision they were taken with, so the collection cannot supply it.
 */
const getEnvironmentDocumentEditor = cachedByCollectionScope((organizationSlug, scope) => {
  const environments = getEnvironmentsCollection(organizationSlug, scope);
  const tails = new Map<string, Promise<string | null>>();
  const action = createOptimisticAction<QueuedEdit>({
    onMutate: ({ environmentId, apply }) => {
      environments.update(environmentId, (draft) => apply(draft.intent));
    },
    mutationFn: async ({ environmentId, revision, save, failureMessage, afterSave }) => {
      const previous = tails.get(environmentId);
      const saved = (async () => {
        const result = await save((previous ? await previous : null) ?? revision);
        await environments.writeCommitted(result.data);
        await afterSave?.();
        return result.data.revision;
      })();
      const tail = saved.catch(() => null);
      tails.set(environmentId, tail);
      void tail.then(() => { if (tails.get(environmentId) === tail) tails.delete(environmentId); });
      try {
        await saved;
      } catch (error) {
        toast.error(error instanceof Error ? error.message : failureMessage);
        throw error;
      }
    },
  });
  return (edit: EnvironmentDocumentEdit) => {
    const document = environments.get(edit.environmentId);
    if (!document) throw new Error("Environment is not loaded.");
    const transaction = action({ ...edit, revision: document.revision });
    // The failure is already toasted; observing it keeps fire-and-forget callers free of unhandled rejections.
    transaction.isPersisted.promise.catch(() => {});
    return transaction;
  };
});

export function editEnvironmentDocument(organizationSlug: string, scope: CollectionScope, edit: EnvironmentDocumentEdit) {
  return getEnvironmentDocumentEditor(organizationSlug, scope)(edit);
}

export function useEnvironmentDocumentEditor(organizationSlug: string) {
  return getEnvironmentDocumentEditor(organizationSlug, useCollectionScope());
}
