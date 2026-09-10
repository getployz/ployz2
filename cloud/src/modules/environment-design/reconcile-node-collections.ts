import { reconcileCollection } from "#/collections/query-collection";
import type { CollectionScope } from "#/collections/scope";
import { getEnvironmentsCollection, getRawServicesCollection, getRawEnvironmentResourcesCollection,
  getResourceLineagesCollection, getCanvasPositionsCollection, getEnvironmentNodeIntroductionsCollection } from "#/collections/collections";

/** Node commands update both the Working State and the identities used by joined views. */
export async function reconcileNodeCollections(organizationSlug: string, scope: CollectionScope) {
  await Promise.all([
    reconcileCollection(getEnvironmentsCollection(organizationSlug, scope)),
    reconcileCollection(getRawServicesCollection(organizationSlug, scope)),
    reconcileCollection(getRawEnvironmentResourcesCollection(organizationSlug, scope)),
    reconcileCollection(getResourceLineagesCollection(organizationSlug, scope)),
    reconcileCollection(getCanvasPositionsCollection(organizationSlug, scope)),
    reconcileCollection(getEnvironmentNodeIntroductionsCollection(organizationSlug, scope)),
  ]);
}
