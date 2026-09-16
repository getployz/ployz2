import type { DeploymentTriggerOrigin } from "#/modules/deployments/deployment";

export type QueueOccupant = {
  id: string;
  savedStateSnapshotId: string;
  triggerOrigin: DeploymentTriggerOrigin;
  inngestRunId: string | null;
};

export type QueueAdmissionRequest = {
  triggerOrigin: DeploymentTriggerOrigin;
  savedStateSnapshotId: string;
};

export type QueueWrite =
  | { kind: "insert" }
  | { kind: "refresh_automated" }
  | { kind: "leave_unchanged" }
  | { kind: "refuse_manual" };

export function decideQueueWrite(
  occupant: QueueOccupant | null,
  request: QueueAdmissionRequest,
): QueueWrite {
  if (occupant === null) return { kind: "insert" };
  switch (request.triggerOrigin.origin) {
    case "manual":
      return { kind: "refuse_manual" };
    case "github":
    case "first_connect":
      if (occupant.triggerOrigin.origin === "manual") {
        return { kind: "leave_unchanged" };
      }
      if (occupant.inngestRunId !== null) {
        return { kind: "leave_unchanged" };
      }
      return { kind: "refresh_automated" };
    default: {
      const _never: never = request.triggerOrigin;
      return _never;
    }
  }
}
