import "@tanstack/react-start/server-only";

import { randomUUID } from "node:crypto";
import { and, eq, inArray, sql } from "drizzle-orm";
import { Effect } from "effect";
import { environment } from "#/modules/project/tables";
import { service, variable, variableSecret, environmentResource, environmentVariableGroup } from "./tables";
import { canonicalizeSavedEnvironmentIntent, compileSavedEnvironmentIntent, parseDashboardEnvironmentIntent, redactSavedEnvironmentIntent, type SavedEnvironmentIntent } from "./saved-intent";
import { Database } from "#/server/database.server";
import { Conflict, NotFound } from "#/server/public-error";

export type EnvironmentDocument = typeof environment.$inferSelect;
export type CurrentEnvironmentSnapshotProjection = ReturnType<typeof compileSavedEnvironmentIntent> & {
  revisionMarkers: string[];
};

export const decodeEnvironmentDocument = Effect.fn("EnvironmentDesign.decodeEnvironmentDocument")(
  (value: SavedEnvironmentIntent) => Effect.try({
    try: () => parseDashboardEnvironmentIntent(value),
    catch: () => new Conflict({ message: "The authored Environment document is invalid." }),
  }),
);

export const loadEnvironmentDocument = Effect.fn("EnvironmentDesign.loadEnvironmentDocument")(
  function* (environmentId: string, forUpdate = false) {
    const { drizzle } = yield* Database;
    const query = drizzle.select().from(environment).where(eq(environment.id, environmentId));
    const [document] = yield* (forUpdate ? query.for("update") : query);
    if (!document) return yield* new NotFound({ message: "Environment not found." });
    const intent = yield* decodeEnvironmentDocument(document.intent);
    if (intent.environmentSlug !== document.namespace) {
      return yield* new Conflict({ message: "Environment identity does not match its document." });
    }
    return { ...document, intent };
  },
);

export const requireDocumentRevision = Effect.fn("EnvironmentDesign.requireDocumentRevision")(
  function* (document: Pick<EnvironmentDocument, "revision">, revision: string) {
    if (document.revision !== revision) return yield* new Conflict({
      message: "Working State changed while this edit was being saved. Review the latest values and try again.",
    });
  },
);

/** One authored write. Call inside the actor's mutation transaction. */
export const writeEnvironmentDocument = Effect.fn("EnvironmentDesign.writeEnvironmentDocument")(
  function* (document: EnvironmentDocument, candidate: SavedEnvironmentIntent) {
    const { drizzle } = yield* Database;
    const intent = canonicalizeSavedEnvironmentIntent(yield* decodeEnvironmentDocument(candidate));
    if (intent.environmentSlug !== document.namespace) {
      return yield* new Conflict({ message: "An edit cannot change the Environment identity." });
    }
    // Identity rows scope private values and history; they contain no authored settings.
    const identities = yield* drizzle.select().from(service).where(eq(service.environmentId, document.id));
    if (intent.services.some((node) => !identities.some((identity) => identity.id === node.id && identity.lineageId === node.lineageId))) {
      return yield* new Conflict({ message: "A service does not belong to this Environment." });
    }
    if (intent.services.some(node => node.config.source.type === "image" &&
      node.config.source.credentials.type === "configured" && node.config.source.credentials.credentialId !== node.id)) {
      return yield* new Conflict({ message: "Registry credentials do not belong to this service." });
    }
    const resources = yield* drizzle.select().from(environmentResource).where(eq(environmentResource.environmentId, document.id));
    const groups = yield* drizzle.select().from(environmentVariableGroup).where(eq(environmentVariableGroup.environmentId, document.id));
    if (intent.volumes.some((node) => !resources.some((identity) => identity.id === node.resourceId && identity.lineageId === node.resourceLineageId && identity.implementationType === "volume")) ||
      intent.variableGroups.some((node) => !resources.some((identity) => identity.id === node.resourceId && identity.lineageId === node.resourceLineageId && identity.implementationType === "variable_group" && identity.variableGroupId === node.variableGroupId) || !groups.some((identity) => identity.id === node.variableGroupId && identity.lineageId === node.variableGroupLineageId))) {
      return yield* new Conflict({ message: "A resource does not belong to this Environment." });
    }
    const variableOwners = [
      ...intent.services.flatMap((node) => node.variables.map((value) => ({ id: value.id, environmentId: document.id, serviceId: node.id, variableGroupId: null }))),
      ...intent.variableGroups.flatMap((node) => node.variables.map((value) => ({ id: value.id, environmentId: document.id, serviceId: null, variableGroupId: node.variableGroupId }))),
    ];
    if (variableOwners.length) {
      yield* drizzle.insert(variable).values(variableOwners).onConflictDoNothing();
      const storedOwners = yield* drizzle.select().from(variable).where(and(eq(variable.environmentId, document.id), inArray(variable.id, variableOwners.map((owner) => owner.id))));
      if (variableOwners.some((owner) => !storedOwners.some((stored) => stored.id === owner.id && stored.serviceId === owner.serviceId && stored.variableGroupId === owner.variableGroupId))) {
        return yield* new Conflict({ message: "A variable identity belongs to a different owner." });
      }
    }
    const secrets = [...intent.services.flatMap((node) => node.variables), ...intent.variableGroups.flatMap((node) => node.variables)]
      .flatMap((variable) => variable.value.kind === "secret" && variable.value.encryptedValue
        ? [{ environmentId: document.id, variableId: variable.id, encryptedValue: variable.value.encryptedValue }]
        : []);
    if (secrets.length) yield* drizzle.insert(variableSecret).values(secrets).onConflictDoUpdate({
      target: variableSecret.variableId,
      set: { encryptedValue: sql`excluded.encrypted_value` },
      setWhere: eq(variableSecret.environmentId, document.id),
    });
    const [written] = yield* drizzle.update(environment).set({
      intent: redactSavedEnvironmentIntent(intent), revision: randomUUID(), updatedAt: new Date(),
    }).where(and(eq(environment.id, document.id), eq(environment.revision, document.revision))).returning();
    if (!written) return yield* new Conflict({ message: "Working State changed while this edit was being saved." });
    return written;
  },
);

/** Ciphertext is private storage, captured into Saved; it is never an editable browser document. */
export const loadCurrentEnvironmentState = Effect.fn("EnvironmentDesign.loadCurrentEnvironmentState")(
  function* (environmentId: string) {
    const { drizzle } = yield* Database;
    const document = yield* loadEnvironmentDocument(environmentId);
    const intent = structuredClone(document.intent);
    const variables = [...intent.services.flatMap((node) => node.variables), ...intent.variableGroups.flatMap((node) => node.variables)];
    const secretIds = variables.filter((variable) => variable.value.kind === "secret").map((variable) => variable.id);
    const secrets = secretIds.length ? yield* drizzle.select().from(variableSecret)
      .where(and(eq(variableSecret.environmentId, environmentId), inArray(variableSecret.variableId, secretIds))) : [];
    const byId = new Map(secrets.map((secret) => [secret.variableId, secret.encryptedValue]));
    for (const variable of variables) {
      if (variable.value.kind !== "secret") continue;
      const encryptedValue = byId.get(variable.id);
      if (!encryptedValue) return yield* new Conflict({ message: "A sealed variable has no private value." });
      variable.value.encryptedValue = encryptedValue;
    }
    return {
      document,
      intent,
      projection: {
        ...compileSavedEnvironmentIntent({ environmentId, intent }),
        revisionMarkers: [`environment:${environmentId}:${document.revision}`],
      } satisfies CurrentEnvironmentSnapshotProjection,
    };
  },
);

export const loadCurrentEnvironmentSnapshotProjection = Effect.fn("EnvironmentDesign.loadCurrentEnvironmentSnapshotProjection")(
  function* (environmentId: string) { return (yield* loadCurrentEnvironmentState(environmentId)).projection; },
);
