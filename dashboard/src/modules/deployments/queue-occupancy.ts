import type { DeploymentTriggerOrigin } from "#/modules/deployments/deployment";

export type QueueOccupant = {
  triggerOrigin: DeploymentTriggerOrigin;
  inngestRunId: string | null;
};

export type QueueWriteKind =
  | "insert"
  | "refresh_automated"
  | "leave_unchanged"
  | "refuse_manual";

export function decideQueueWrite(
  occupant: QueueOccupant | null,
  triggerOrigin: DeploymentTriggerOrigin,
): QueueWriteKind {
  if (occupant === null) return "insert";
  switch (triggerOrigin.origin) {
    case "manual":
      return "refuse_manual";
    case "github":
    case "first_connect":
      if (occupant.triggerOrigin.origin === "manual") {
        return "leave_unchanged";
      }
      if (occupant.inngestRunId !== null) {
        return "leave_unchanged";
      }
      return "refresh_automated";
    default: {
      const _never: never = triggerOrigin;
      return _never;
    }
  }
}
