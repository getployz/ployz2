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
      decideQueueWrite(null, manual),
    ).toEqual({ kind: "insert" });
  });

  it("refuses a second manual Deploy while any attempt is queued", () => {
    expect(
      decideQueueWrite(occupant(github), manual),
    ).toEqual({ kind: "refuse_manual", occupant: occupant(github) });
    expect(
      decideQueueWrite(occupant(manual), manual),
    ).toEqual({ kind: "refuse_manual", occupant: occupant(manual) });
  });

  it("leaves a queued manual attempt unchanged for automated triggers", () => {
    expect(
      decideQueueWrite(occupant(manual), github),
    ).toEqual({ kind: "leave_unchanged", occupant: occupant(manual) });
    expect(
      decideQueueWrite(occupant(manual), firstConnect),
    ).toEqual({ kind: "leave_unchanged", occupant: occupant(manual) });
  });

  it("refreshes an unowned automated occupant from a later automated admit", () => {
    expect(
      decideQueueWrite(occupant(github), github),
    ).toEqual({ kind: "refresh_automated", occupant: occupant(github) });
  });

  it("does not refresh an automated occupant after Inngest owns it", () => {
    const owned = occupant(github, { inngestRunId: "run-1" });
    expect(
      decideQueueWrite(owned, github),
    ).toEqual({ kind: "leave_unchanged", occupant: owned });
  });
});
