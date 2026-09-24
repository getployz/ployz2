import { toast } from "sonner";
import { createOptimisticAction } from "@tanstack/react-db";
import { getEnvironmentsCollection } from "#/collections/collections";
import { cachedByCollectionScope, type CollectionScope } from "#/collections/scope";
import { useCollectionScope } from "#/collections/use-collection-scope";
import type { environment as schemaEnvironment } from "#/modules/project/tables";
import type { SavedEnvironmentIntent } from "./saved-intent";

type SavedEnvironment = typeof schemaEnvironment.$inferSelect;
type Persistable = { isPersisted: { promise: Promise<unknown> } };

export type EnvironmentDocumentEdit = {
  environmentId: string;
  /**
   * Applied to the in-memory document immediately; rolled back if saving fails.
   * Skip silently when the target is gone: a concurrent change removed it, and the saved document reconciles.
   */
  apply: (intent: SavedEnvironmentIntent) => void;
  /** Saves against `revision` and returns the saved environment. */
  save: (revision: string) => Promise<{ data: SavedEnvironment }>;
  /** Toasted when saving fails and the edit rolls back. */
  failureMessage: string;
  /** Runs after the saved environment is committed, before the edit counts as persisted. */
  afterSave?: () => Promise<void>;
};

type QueuedEdit = EnvironmentDocumentEdit & { revision: string };

/** Failures are toasted where they happen; observing them keeps fire-and-forget callers free of unhandled rejections. */
export function observeFailure<T extends Persistable>(transaction: T): T {
  transaction.isPersisted.promise.catch(() => {});
  return transaction;
}

/**
 * The one way to change an environment document: apply now, save in the background,
 * roll back and toast on failure. Callers do not wait.
 *
 * The server accepts only the current revision, so saves to one environment run in
 * order, each against the latest revision the queue has seen. Optimistic snapshots
 * keep the revision they were taken with, so the collection cannot supply it.
 */
const getEnvironmentDocumentEditor = cachedByCollectionScope((organizationSlug, scope) => {
  const environments = getEnvironmentsCollection(organizationSlug, scope);
  const tails = new Map<string, Promise<string>>();
  const action = createOptimisticAction<QueuedEdit>({
    onMutate: ({ environmentId, apply }) => {
      environments.update(environmentId, (draft) => apply(draft.intent));
    },
    mutationFn: async ({ environmentId, revision, save, failureMessage, afterSave }) => {
      const base = tails.get(environmentId) ?? Promise.resolve(revision);
      const saved = (async () => {
        const result = await save(await base);
        await environments.writeCommitted(result.data);
        await afterSave?.();
        return result.data.revision;
      })();
      // A failed save leaves the server on the revision it started from.
      const tail = saved.catch(() => base);
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
  return {
    edit(edit: EnvironmentDocumentEdit) {
      const document = environments.get(edit.environmentId);
      if (!document) throw new Error("Environment is not loaded.");
      return observeFailure(action({ ...edit, revision: document.revision }));
    },
    /** Resolves once every queued save for the environment has settled. */
    async settled(environmentId: string) {
      await tails.get(environmentId);
    },
  };
});

export function editEnvironmentDocument(organizationSlug: string, scope: CollectionScope, edit: EnvironmentDocumentEdit) {
  return getEnvironmentDocumentEditor(organizationSlug, scope).edit(edit);
}

/**
 * For edits that compute something asynchronously first (hashing a variable value, say).
 * A failure while preparing is toasted here; a failed save is toasted by the editor.
 */
export function editEnvironmentDocumentAfter(organizationSlug: string, scope: CollectionScope,
  prepare: () => Promise<EnvironmentDocumentEdit>, failureMessage: string): Persistable {
  const promise = (async () => {
    const edit = await prepare().catch((error: Error) => {
      toast.error(error.message || failureMessage);
      throw error;
    });
    await editEnvironmentDocument(organizationSlug, scope, edit).isPersisted.promise;
  })();
  return observeFailure({ isPersisted: { promise } });
}

export function useEnvironmentDocumentEditor(organizationSlug: string) {
  const editor = getEnvironmentDocumentEditor(organizationSlug, useCollectionScope());
  return (edit: EnvironmentDocumentEdit) => editor.edit(edit);
}

/** Commands that read the document's revision or working state wait here for queued saves first. */
export function useEnvironmentDocumentSettled(organizationSlug: string) {
  const editor = getEnvironmentDocumentEditor(organizationSlug, useCollectionScope());
  return (environmentId: string) => editor.settled(environmentId);
}
