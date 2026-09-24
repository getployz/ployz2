import { sourceTablesOf } from "#/collections/change-sources";
import { collectionReadInput, type CollectionName } from "#/collections/read.contract";

/** Every Org Store table the collection read serves; fixtures seed all of them so the gate never reads the network. */
export const orgStoreTableNames = collectionReadInput.fields.table.literals.filter((table) => table !== "github_repository_cache");

/** The Query data a table's collection caches: change-log collections keep their rows beside a cursor. */
export function orgStoreSeed<Row>(table: CollectionName, rows: Row[]) {
  return sourceTablesOf(table).length > 0 ? { rows, cursor: null } : rows;
}
