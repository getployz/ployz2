import {
  parseServiceConfig,
  parseServiceSetting,
  type ServiceSettingInput,
  type ServiceConfig,
} from "@ployz/sdk/config";
import { Effect, Schema, SchemaGetter, SchemaIssue } from "effect";

import { encryptedSecretValueSchema, variableValuePartsSchema } from "./variables";
import { Uuid, decodeStrict, type DeepMutable } from "./schema";

const envSourceSchema = Schema.Struct({
  kind: Schema.Literal("variable_group"), resourceId: Uuid, resourceName: Schema.NonEmptyString,
  variableGroupId: Uuid, key: Schema.NonEmptyString,
});
const envValueSchema = Schema.Union([
  Schema.Struct({ kind: Schema.Literal("literal"), value: Schema.String,
    source: Schema.optionalKey(envSourceSchema), parts: Schema.optionalKey(variableValuePartsSchema) }),
  Schema.Struct({ kind: Schema.Literal("secret"), variableId: Schema.optionalKey(Uuid),
    encryptedValue: Schema.optionalKey(encryptedSecretValueSchema), fingerprint: Schema.NonEmptyString,
    source: Schema.optionalKey(envSourceSchema), interpolated: Schema.optionalKey(Schema.Boolean) }),
]);
const attachmentsSchema = Schema.Array(Schema.Struct({ variableGroupId: Uuid, sortOrder: Schema.Int }));
export type DashboardServiceConfig = Omit<ServiceConfig, "env"> & {
  env: Record<string, DeepMutable<typeof envValueSchema.Type>>;
  variableGroupAttachments: Array<{ variableGroupId: string; sortOrder: number }>;
};

/** Core compares ordinary Service values; group expressions remain Dashboard display values here. */
export function toCoreServiceConfig(value: DashboardServiceConfig): ServiceConfig {
  const { variableGroupAttachments: _attachments, env, ...config } = value;
  return { ...config, env: Object.fromEntries(Object.entries(env).map(([key, entry]) => {
    const { source: _source, ...plain } = entry;
    if (plain.kind === "secret" || !plain.parts) return [key, plain];
    const parts = plain.parts.flatMap((part) => part.kind === "ref" && part.owner.scope === "variable_group" ? [] : [part]);
    // Group expressions are compared by their rendered value, never sent as Core reference owners.
    if (parts.length !== plain.parts.length) return [key, { kind: "literal" as const, value: plain.value }];
    return [key, { ...plain, parts: parts as Extract<ServiceConfig["env"][string], { kind: "literal" }>["parts"] }];
  })) };
}

/** Validate Dashboard extensions separately from Core-owned settings. */
export function parseDashboardServiceConfig(value: unknown): DashboardServiceConfig {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Invalid Service configuration.");
  const { variableGroupAttachments = [], env = {}, ...settings } = value as Record<string, unknown>;
  const attachments = structuredClone(decodeStrict(attachmentsSchema, variableGroupAttachments)) as DashboardServiceConfig["variableGroupAttachments"];
  const parsedEnv = structuredClone(decodeStrict(Schema.Record(Schema.String, envValueSchema), env)) as DashboardServiceConfig["env"];
  const config = parseServiceConfig({ ...settings, env: {} });
  return { ...config, env: parsedEnv, variableGroupAttachments: attachments };
}

/** Effect owns the input envelope; Rust owns setting admission and normalization. */
export function sharedSchema<T>(parse: <Input>(value: Input) => T) {
  const decoded = Schema.declare<T>((value): value is T => {
    try { parse(value); return true; } catch { return false; }
  });
  return decoded.pipe(Schema.decodeTo(decoded, {
    decode: SchemaGetter.transformOrFail((value, options) => Effect.try({
      try: () => parse(value),
      catch: (cause) => new SchemaIssue.InvalidValue({
        message: cause instanceof Error ? cause.message : "Invalid service setting",
      }, undefined, options),
    })),
    encode: SchemaGetter.transform((value) => value),
  }));
}

export function serviceFieldSchema<Field extends ServiceSettingInput["field"]>(field: Field) {
  return sharedSchema((value) => parseServiceSetting(field, value));
}

export const sharedServiceConfigSchema = sharedSchema(parseDashboardServiceConfig);
export const savedServiceConfigSchema = sharedSchema((value) => {
  const { env: _env, mounts: _mounts, variableGroupAttachments: _variableGroupAttachments, ...settings } = parseDashboardServiceConfig(value);
  return settings;
});
