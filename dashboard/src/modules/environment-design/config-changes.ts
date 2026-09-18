import { compareServiceSettings, type ServiceSettingChange } from "@ployz/sdk/config";
import { toCoreServiceConfig, type DashboardServiceConfig } from "./service-config";
import { variableGroupConfigSchema, type VariableGroupConfig } from "./variable-group-config";
import { decodeStrict } from "./schema";

export function compareDashboardServiceSettings(current: DashboardServiceConfig, baseline: DashboardServiceConfig | null) {
  const changes = compareServiceSettings(toCoreServiceConfig(current), baseline ? toCoreServiceConfig(baseline) : null);
  const beforeAttachments = baseline?.variableGroupAttachments ?? [];
  const afterAttachments = current.variableGroupAttachments;
  if (JSON.stringify(beforeAttachments) !== JSON.stringify(afterAttachments)) changes.push({
    path: "variableGroupAttachments", before: beforeAttachments, after: afterAttachments,
    kind: baseline ? "update" : "add", canRestore: baseline !== null,
  });
  return changes.map((change) => ({ ...change,
    derivedFrom: change.path.startsWith("env.")
      ? current.env[change.path.slice(4)]?.source ?? baseline?.env[change.path.slice(4)]?.source
      : undefined,
  }));
}

/** Group comparison stays in Dashboard; Core compares Volume settings. */
export function compareVariableGroupSettings(current: unknown, baseline: unknown): ServiceSettingChange[] {
  const after = decodeStrict(variableGroupConfigSchema, current);
  const before = baseline ? decodeStrict(variableGroupConfigSchema, baseline) : null;
  const changes: ServiceSettingChange[] = [];
  if (!before || before.name !== after.name) changes.push({
    path: before ? "name" : "node", kind: before ? "update" : "add",
    before: before?.name ?? null, after: after.name, canRestore: before === null,
  });
  const index = (config: VariableGroupConfig | null) => new Map(config?.variables.map((v) => [v.key, v]));
  const oldVariables = index(before);
  const newVariables = index(after);
  const comparable = (variable: VariableGroupConfig["variables"][number] | undefined) => variable ? {
    description: variable.description, exported: variable.exported,
    value: variable.value.type === "sealed"
      ? { type: "sealed", hasValue: true, fingerprint: variable.value.fingerprint }
      : variable.value,
  } : null;
  const display = (variable: VariableGroupConfig["variables"][number] | undefined) =>
    !variable ? null : variable.value.type === "sealed" ? { kind: "secret" } : variable.value.value;
  for (const key of [...new Set([...oldVariables.keys(), ...newVariables.keys()])].sort()) {
    const oldVariable = oldVariables.get(key);
    const newVariable = newVariables.get(key);
    if (JSON.stringify(comparable(oldVariable)) === JSON.stringify(comparable(newVariable))) continue;
    changes.push({ path: `variables.${key}`, kind: !oldVariable ? "add" : !newVariable ? "remove" : "update",
      before: display(oldVariable), after: display(newVariable), canRestore: false });
  }
  return changes;
}
