import type { CollectionName } from "./read.contract";

/**
 * The one map from a change-log source table to the Org Store collections it feeds.
 * The source table's trigger logs the collection key, so collections sharing a table share its key.
 */
export const changeSources = new Map<string, readonly CollectionName[]>([
  ["service", ["service"]],
]);

export function sourceTablesOf(collection: CollectionName) {
  return [...changeSources].filter(([, collections]) => collections.includes(collection)).map(([table]) => table);
}

export function collectionsOf(sourceTables: Iterable<string>) {
  return [...new Set([...sourceTables].flatMap((table) => changeSources.get(table) ?? []))];
}
