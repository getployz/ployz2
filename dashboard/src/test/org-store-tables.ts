import { collectionReadInput } from "#/collections/read.contract";

/** Every Org Store table the collection read serves; fixtures seed all of them so the gate never reads the network. */
export const orgStoreTableNames = collectionReadInput.fields.table.literals.filter((table) => table !== "github_repository_cache");

/** The Query data an Org Store collection caches: its rows beside a change cursor. */
export function orgStoreSeed<Row>(rows: Row[]) {
  return { rows, cursor: null };
}
