import type { DeploymentTriggerOrigin } from "#/modules/deployments/deployment";

export type QueueOccupant = {
  id: string;
  savedStateSnapshotId: string;
  triggerOrigin: DeploymentTriggerOrigin;
  inngestRunId: string | null;
};

export type QueueWrite<Occupant extends QueueOccupant = QueueOccupant> =
  | { kind: "insert" }
  | { kind: "refresh_automated"; occupant: Occupant }
  | { kind: "leave_unchanged"; occupant: Occupant }
  | { kind: "refuse_manual"; occupant: Occupant };

export function decideQueueWrite<Occupant extends QueueOccupant>(
  occupant: Occupant | null,
  triggerOrigin: DeploymentTriggerOrigin,
): QueueWrite<Occupant> {
  if (occupant === null) return { kind: "insert" };
  switch (triggerOrigin.origin) {
    case "manual":
      return { kind: "refuse_manual", occupant };
    case "github":
    case "first_connect":
      if (occupant.triggerOrigin.origin === "manual") {
        return { kind: "leave_unchanged", occupant };
      }
      if (occupant.inngestRunId !== null) {
        return { kind: "leave_unchanged", occupant };
      }
      return { kind: "refresh_automated", occupant };
    default: {
      const _never: never = triggerOrigin;
      return _never;
    }
  }
}
