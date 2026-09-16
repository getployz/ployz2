import { describe, expect, it, vi } from "vitest";
import {
  canvasPublicationInput,
  submitCanvasPublication,
} from "./canvas-publication-submission";
import {
  reviewedPublicationSchema,
  type DestructiveVolumeReview,
} from "#/modules/deployments/deployment-contract";
import { isValid } from "#/modules/environment-design/schema";

const params = {
  organizationSlug: "acme",
  projectSlug: "api",
  environmentSlug: "production",
};
const savedStateBasis = { kind: "no_saved_state" as const };
const fingerprint = `environment-working-state-v1:${"a".repeat(64)}`;
const volumeReview: DestructiveVolumeReview = {
  target: {
    version: 1,
    resourceId: "00000000-0000-4000-8000-000000000001",
    namespaceId: "production",
    volumeName: "vol-data",
    machineId: "a".repeat(32),
  },
  evidence: {
    version: 1,
    fingerprint: "identity-fingerprint",
    reviewedAt: "2026-09-16T00:00:00.000Z",
    evidence: {
      namespaceId: "production",
      volumeName: "vol-data",
      machineId: "a".repeat(32),
      kind: { kind: "plain" },
      availability: {
        status: "available",
        usedBytes: 0,
        lastWriteUnixSeconds: 0,
      },
      referencingServices: [],
    },
  },
};

describe("canvas publication submission", () => {
  it("sends the same complete review for Save and Deploy", () => {
    const review = {
      message: "Publish removals",
      savedStateBasis,
      reviewedWorkingStateFingerprint: fingerprint,
      destructiveServiceIds: ["00000000-0000-4000-8000-000000000002"],
      destructiveVolumeReviews: [volumeReview],
    };
    const save = canvasPublicationInput(params, {
      kind: "save",
      ...review,
    });
    const deploy = canvasPublicationInput(params, {
      kind: "deploy",
      ...review,
    });
    const wireReview = {
      savedStateBasis,
      workingStateFingerprint: fingerprint,
      destructiveServiceIds: review.destructiveServiceIds,
      destructiveVolumeReviews: review.destructiveVolumeReviews,
    };

    expect(save).toEqual({
      ...params,
      intent: "save",
      message: review.message,
      review: wireReview,
    });
    expect(deploy).toEqual({
      ...params,
      intent: "manual_deploy",
      message: review.message,
      review: wireReview,
    });
    expect(isValid(reviewedPublicationSchema, save)).toBe(true);
    expect(isValid(reviewedPublicationSchema, deploy)).toBe(true);
  });

  it("surfaces a conflict without reconciling or resubmitting", async () => {
    const submit = vi.fn(async () => {
      throw Object.assign(
        new Error("Working State changed after this action was reviewed."),
        { _tag: "Conflict" },
      );
    });
    const reconcile = vi.fn();

    await expect(
      submitCanvasPublication({
        submit,
        reconcile,
        data: canvasPublicationInput(params, {
          kind: "deploy",
          message: "Stale",
          savedStateBasis,
          reviewedWorkingStateFingerprint: fingerprint,
          destructiveServiceIds: [],
          destructiveVolumeReviews: [],
        }),
      }),
    ).rejects.toMatchObject({
      _tag: "Conflict",
      message: "Working State changed after this action was reviewed.",
    });
    expect(submit).toHaveBeenCalledTimes(1);
    expect(reconcile).not.toHaveBeenCalled();
  });
});
