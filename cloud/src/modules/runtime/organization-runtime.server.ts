import "@tanstack/react-start/server-only";
import type { Connection } from "@ployz/sdk";
import {
  Context,
  Effect,
  Layer,
  type Scope,
} from "effect";
import {
  Ployz,
  type PloyzProviderError,
  type PloyzSession,
} from "#/modules/runtime/ployz.server";
import {
  loadOrganizationConnections,
} from "#/modules/machines/enrollment.server";
import { Database } from "#/server/database.server";
import { SecretEncryption } from "#/utils/encrypted-secret.server";

export type ConnectedRuntimeClient = PloyzSession;

export type ScopedRuntimeClientSession =
  | { readonly status: "connected"; readonly connected: ConnectedRuntimeClient }
  | { readonly status: "no_connection" }
  | {
      readonly status: "unreachable";
      readonly error: PloyzProviderError | null;
    };

export interface OrganizationRuntimeService {
  readonly open: (
    organizationId: string,
  ) => Effect.Effect<ScopedRuntimeClientSession, Error, Scope.Scope>;
}

export class OrganizationRuntime extends Context.Service<
  OrganizationRuntime,
  OrganizationRuntimeService
>()("ployz/OrganizationRuntime") {}

type LoadConnections = (
  organizationId: string,
) => Effect.Effect<
  { readonly kind: "missing" } | { readonly kind: "ready"; readonly connections: readonly Connection[] },
  Error
>;

export function makeOrganizationRuntimeLayer(loadConnections: LoadConnections) {
  return Layer.effect(
    OrganizationRuntime,
    Effect.gen(function* () {
      const ployz = yield* Ployz;
      return {
        open: Effect.fn("OrganizationRuntime.open")(function* (
          organizationId: string,
        ) {
          const access = yield* loadConnections(organizationId);
          if (access.kind === "missing") {
            return { status: "no_connection" as const };
          }
          if (access.connections.length === 0) {
            return { status: "unreachable" as const, error: null };
          }
          return yield* ployz.connect({ connections: access.connections }).pipe(
            Effect.map((connected) => ({
              status: "connected" as const,
              connected,
            })),
            Effect.catch((error) =>
              Effect.succeed({
                status: "unreachable" as const,
                error,
              }),
            ),
          );
        }),
      } satisfies OrganizationRuntimeService;
    }),
  );
}

export const OrganizationRuntimeLive = Layer.unwrap(
  Effect.gen(function* () {
    const database = yield* Database;
    const encryption = yield* SecretEncryption;
    return makeOrganizationRuntimeLayer((organizationId) =>
      loadOrganizationConnections(organizationId).pipe(
        Effect.provideService(Database, database),
        Effect.provideService(SecretEncryption, encryption),
      ),
    );
  }),
);
