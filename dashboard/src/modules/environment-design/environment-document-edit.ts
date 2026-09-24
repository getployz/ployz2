import { toast } from "sonner";
import { createOptimisticAction } from "@tanstack/react-db";
import { getEnvironmentsCollection } from "#/collections/collections";
import { observeFailure, type Persistable } from "#/collections/query-collection";
import { cachedByCollectionScope, type CollectionScope } from "#/collections/scope";
import { useCollectionScope } from "#/collections/use-collection-scope";
import type { environment as schemaEnvironment } from "#/modules/project/tables";
import type { SavedEnvironmentIntent } from "./saved-intent";

type SavedEnvironment = typeof schemaEnvironment.$inferSelect;

/** A save against the environment's current revision, run in queue order. */
export type EnvironmentDocumentSave = {
  environmentId: string;
  /** Saves against `revision` and returns the saved environment. */
  save: (revision: string) => Promise<{ data: SavedEnvironment }>;
  /** Toasted when saving fails. */
  failureMessage: string;
  /** Runs after the saved environment is committed, before the save counts as persisted. */
  afterSave?: () => Promise<void>;
};

export type EnvironmentDocumentEdit = EnvironmentDocumentSave & {
  /**
   * Applied to the in-memory document immediately; rolled back if saving fails.
   * Must change the document (an unchanged edit never saves). Skip silently when the
   * target is gone: a concurrent change removed it, and the saved document reconciles.
   */
  apply: (intent: SavedEnvironmentIntent) => void;
};

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
  const inFlight = new Map<string, Set<Promise<unknown>>>();

  const notLoaded = () => new Error("Environment is not loaded.");

  async function runInQueue({ environmentId, save, failureMessage, afterSave }: EnvironmentDocumentSave, revision: string) {
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
  }

  function track(environmentId: string, persisted: Promise<unknown>) {
    const set = inFlight.get(environmentId) ?? new Set();
    inFlight.set(environmentId, set);
    set.add(persisted);
    void persisted.catch(() => {}).finally(() => set.delete(persisted));
  }

  const action = createOptimisticAction<EnvironmentDocumentEdit & { revision: string }>({
    onMutate: ({ environmentId, apply }) => {
      environments.update(environmentId, (draft) => apply(draft.intent));
    },
    mutationFn: ({ revision, ...edit }) => runInQueue(edit, revision),
  });

  /** A document that is not loaded fails like a save: toasted, and observed. */
  function failed(error: Error): Persistable {
    toast.error(error.message);
    return observeFailure({ isPersisted: { promise: Promise.reject(error) } });
  }

  return {
    edit(edit: EnvironmentDocumentEdit): Persistable {
      const revision = environments.get(edit.environmentId)?.revision;
      if (!revision) return failed(notLoaded());
      const transaction = observeFailure(action({ ...edit, revision }));
      track(edit.environmentId, transaction.isPersisted.promise);
      return transaction;
    },
    /** A queued save with nothing to apply up front, for commands like discard. */
    enqueue(save: EnvironmentDocumentSave): Persistable {
      const revision = environments.get(save.environmentId)?.revision;
      if (!revision) return failed(notLoaded());
      const persisted = runInQueue(save, revision);
      track(save.environmentId, persisted);
      return observeFailure({ isPersisted: { promise: persisted } });
    },
    /**
     * Resolves once every queued edit for the environment has finished and its optimistic
     * overlay is gone. Edits still preparing (see `editEnvironmentDocumentAfter`) are not queued yet.
     */
    async settled(environmentId: string) {
      for (let set = inFlight.get(environmentId); set?.size; set = inFlight.get(environmentId)) {
        await Promise.allSettled(set);
      }
    },
  };
});

export function editEnvironmentDocument(organizationSlug: string, scope: CollectionScope, edit: EnvironmentDocumentEdit) {
  return getEnvironmentDocumentEditor(organizationSlug, scope).edit(edit);
}

/**
 * For edits that compute something asynchronously first (hashing a variable value, say).
 * A failure before the save is queued is toasted here; a failed save is toasted by the queue.
 */
export function editEnvironmentDocumentAfter(organizationSlug: string, scope: CollectionScope,
  prepare: () => Promise<EnvironmentDocumentEdit>, failureMessage: string): Persistable {
  const promise = (async () => {
    let edit: EnvironmentDocumentEdit;
    try {
      edit = await prepare();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : failureMessage);
      throw error;
    }
    await editEnvironmentDocument(organizationSlug, scope, edit).isPersisted.promise;
  })();
  return observeFailure({ isPersisted: { promise } });
}

export function useEnvironmentDocumentEditor(organizationSlug: string) {
  const editor = getEnvironmentDocumentEditor(organizationSlug, useCollectionScope());
  return (edit: EnvironmentDocumentEdit) => editor.edit(edit);
}

/** For commands that must run in the save queue (discard) or wait for it (publish). */
export function useEnvironmentDocumentQueue(organizationSlug: string) {
  const editor = getEnvironmentDocumentEditor(organizationSlug, useCollectionScope());
  return {
    enqueue: (save: EnvironmentDocumentSave) => editor.enqueue(save),
    settled: (environmentId: string) => editor.settled(environmentId),
  };
}
