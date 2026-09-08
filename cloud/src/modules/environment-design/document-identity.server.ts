import "@tanstack/react-start/server-only";
import { sql } from "drizzle-orm";
import { environment } from "#/modules/project/tables";

const environmentResourceTable = sql.identifier("environment_resource");
const environmentResourceId = sql.identifier("id");
const environmentResourceEnvironmentId = sql.identifier("environment_id");
const serviceTable = sql.identifier("service");
const serviceId = sql.identifier("id");
const serviceEnvironmentId = sql.identifier("environment_id");

export const volumeIsAuthored = sql<boolean>`exists (
  select 1 from ${environment} document,
  jsonb_array_elements(document.intent->'volumes') node
  where document.id = ${environmentResourceTable}.${environmentResourceEnvironmentId}
    and node->>'resourceId' = ${environmentResourceTable}.${environmentResourceId}::text
)`;
export const serviceIsAuthored = sql<boolean>`exists (
  select 1 from ${environment} document,
  jsonb_array_elements(document.intent->'services') node
  where document.id = ${serviceTable}.${serviceEnvironmentId}
    and node->>'id' = ${serviceTable}.${serviceId}::text
)`;
