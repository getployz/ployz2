import "@tanstack/react-start/server-only";
import { getTableColumns, sql } from "drizzle-orm";
import { volumeRemoveAttempt } from "#/modules/runtime/tables";
import { environmentDeployment, environmentDeploymentEvent } from "./tables";

// The plan the runtime executes (manifest, producers, action policy) stays on the server; no view reads it.
const { deployManifest: _manifest, variableProducers: _producers, serviceActionPolicy: _policy, ...deploymentColumns } =
  getTableColumns(environmentDeployment);

/** The deployment row every browser read returns: the Org Store collection and the history reads map it the same way. */
export const deploymentRowColumns = {
  ...deploymentColumns,
  runtimeProgress: sql<typeof environmentDeployment.$inferSelect.runtimeProgress>`coalesce(
    ${environmentDeployment.runtimeProgress},
    (select progress from ${environmentDeploymentEvent}
     where deployment_id = ${environmentDeployment}.${sql.identifier("id")} order by id desc limit 1)
  )`,
  // Removal retries need a fresh destructive review, so an attempt that staged one cannot retry.
  canRetry: sql<boolean>`${environmentDeployment.status} = 'failed' and not exists (
    select 1 from ${volumeRemoveAttempt}
    where ${volumeRemoveAttempt.environmentDeploymentId} = ${environmentDeployment}.${sql.identifier("id")}
  )`,
};
