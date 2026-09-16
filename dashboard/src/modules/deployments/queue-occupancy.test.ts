import { describe, expect, it } from "vitest";
import {
  decideQueueWrite,
  type QueueOccupant,
} from "#/modules/deployments/queue-occupancy";

const manual = {
  origin: "manual" as const,
  actorId: "actor-1",
};
const github = {
  origin: "github" as const,
  deliveryId: "delivery-1",
  branchEvaluationRevision: 1,
  installationId: 17,
  repositoryId: 42,
};
const firstConnect = {
  origin: "first_connect" as const,
  machineId: "0123456789abcdef0123456789abcdef",
};

function occupant(
  triggerOrigin: QueueOccupant["triggerOrigin"],
  input?: { inngestRunId?: string | null },
): QueueOccupant {
  return {
    id: "attempt-1",
    savedStateSnapshotId: "saved-1",
    triggerOrigin,
    inngestRunId: input?.inngestRunId ?? null,
  };
}

describe("decideQueueWrite", () => {
  it("inserts when the queue slot is vacant", () => {
    expect(
      decideQueueWrite(null, {
        triggerOrigin: manual,
        savedStateSnapshotId: "saved-2",
      }),
    ).toEqual({ kind: "insert" });
  });

  it("refuses a second manual Deploy while any attempt is queued", () => {
    expect(
      decideQueueWrite(occupant(github), {
        triggerOrigin: manual,
        savedStateSnapshotId: "saved-2",
      }),
    ).toEqual({ kind: "refuse_manual" });
    expect(
      decideQueueWrite(occupant(manual), {
        triggerOrigin: manual,
        savedStateSnapshotId: "saved-2",
      }),
    ).toEqual({ kind: "refuse_manual" });
  });

  it("leaves a queued manual attempt unchanged for automated triggers", () => {
    expect(
      decideQueueWrite(occupant(manual), {
        triggerOrigin: github,
        savedStateSnapshotId: "saved-2",
      }),
    ).toEqual({ kind: "leave_unchanged" });
    expect(
      decideQueueWrite(occupant(manual), {
        triggerOrigin: firstConnect,
        savedStateSnapshotId: "saved-2",
      }),
    ).toEqual({ kind: "leave_unchanged" });
  });

  it("refreshes an unowned automated occupant from a later automated admit", () => {
    expect(
      decideQueueWrite(occupant(github), {
        triggerOrigin: github,
        savedStateSnapshotId: "saved-2",
      }),
    ).toEqual({ kind: "refresh_automated" });
  });

  it("does not refresh an automated occupant after Inngest owns it", () => {
    expect(
      decideQueueWrite(occupant(github, { inngestRunId: "run-1" }), {
        triggerOrigin: github,
        savedStateSnapshotId: "saved-2",
      }),
    ).toEqual({ kind: "leave_unchanged" });
  });
});
