import { queryCollectionOptions, type QueryCollectionUtils } from "@tanstack/query-db-collection";
import { BasicIndex, createCollection } from "@tanstack/react-db";
import type { QueryClient } from "@tanstack/react-query";

/** Each owner supplies its request-local QueryClient and authenticated scope. */
export function createApiCollection<T extends object>(input: {
  queryClient: QueryClient;
  queryKey: readonly string[];
  queryFn: (context: { signal: AbortSignal }) => Promise<T[]>;
  getKey: (row: T) => string | number;
}) {
  // Default snapshot retention lets a loader hand data to its consumer after releasing its observer.
  return createCollection(queryCollectionOptions({
    ...input,
    id: input.queryKey.join(":"),
    startSync: false,
    refetchInterval: 15_000,
    refetchOnWindowFocus: "always",
    refetchOnReconnect: "always",
    retry: false,
    autoIndex: "eager",
    defaultIndexType: BasicIndex,
  }));
}

type ApiCollectionReadiness = {
  preload: () => Promise<void>;
  subscribeChanges: (callback: () => void) => { unsubscribe: () => void };
  utils: Pick<QueryCollectionUtils, "refetch" | "isError" | "lastError" | "dataUpdatedAt">;
};

/** Hold an observer only until readiness; failed first reads are not empty snapshots. */
export async function preloadCollection(collection: ApiCollectionReadiness) {
  const retryInitialFailure = collection.utils.isError && collection.utils.dataUpdatedAt === 0;
  const subscription = collection.subscribeChanges(() => {});
  try {
    if (retryInitialFailure) await collection.utils.refetch({ throwOnError: true });
    else await collection.preload();
    if (collection.utils.isError && collection.utils.dataUpdatedAt === 0) {
      throw collection.utils.lastError;
    }
  } finally {
    subscription.unsubscribe();
  }
}

/** Reconciliation also works before any live view has subscribed to the collection. */
export async function reconcileCollection(collection: ApiCollectionReadiness) {
  const subscription = collection.subscribeChanges(() => {});
  try {
    await collection.preload();
    await collection.utils.refetch({ throwOnError: true });
  } finally {
    subscription.unsubscribe();
  }
}
