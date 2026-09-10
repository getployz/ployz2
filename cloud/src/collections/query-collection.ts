import { queryCollectionOptions } from "@tanstack/query-db-collection";
import { BasicIndex, createCollection } from "@tanstack/react-db";
import type { QueryClient } from "@tanstack/react-query";

/** Each owner supplies its request-local QueryClient and authenticated scope. */
export function createApiCollection<T extends object>(input: {
  queryClient: QueryClient;
  queryKey: readonly string[];
  queryFn: (context: { signal: AbortSignal }) => Promise<T[]>;
  getKey: (row: T) => string | number;
}) {
  return createCollection({
    ...queryCollectionOptions({
      ...input,
      id: input.queryKey.join(":"),
      startSync: false,
      gcTime: 1,
      refetchInterval: 15_000,
      refetchOnWindowFocus: "always",
      refetchOnReconnect: "always",
      retry: false,
      autoIndex: "eager",
      defaultIndexType: BasicIndex,
    }),
    // This is DB's separate GC timer; zero disables it. Release unused scopes.
    gcTime: 1,
  });
}
