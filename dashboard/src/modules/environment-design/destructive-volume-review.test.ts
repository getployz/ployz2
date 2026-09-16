import { describe, expect, it } from "vitest";
import {
  getDestructiveVolumeReviewMismatch,
  reviewedPublicationSchema,
  type DestructiveVolumeReview,
} from "#/modules/deployments/deployment-contract";
import {
  destructiveVolumeReviewsSchema,
  resolveSavedVolumeDeletionAuthorizations,
} from "#/modules/environment-design/destructive-volume-review";
import { isValid } from "#/modules/environment-design/schema";

const workingStateFingerprint =
  `environment-working-state-v1:${"a".repeat(64)}`;
const savedStateBasis = { kind: "no_saved_state" as const };

function validPublication(
  review: {
    destructiveServiceIds?: string[];
    destructiveVolumeReviews?: DestructiveVolumeReview[];
  } = {},
) {
  return {
    organizationSlug: "acme",
    projectSlug: "api",
    environmentSlug: "production",
    intent: "save" as const,
    message: null,
    review: {
      savedStateBasis,
      workingStateFingerprint,
      destructiveServiceIds: review.destructiveServiceIds ?? [],
      destructiveVolumeReviews: review.destructiveVolumeReviews ?? [],
    },
  };
}

describe("destructive reviews on deployment admission", () => {
  it("accepts each reviewed Service removal once", () => {
    const valid = isValid(
      reviewedPublicationSchema,
      validPublication({
        destructiveServiceIds: ["00000000-0000-4000-8000-000000000001"],
      }),
    );
    expect(valid).toBe(true);

    const duplicate = isValid(
      reviewedPublicationSchema,
      validPublication({
        destructiveServiceIds: [
          "00000000-0000-4000-8000-000000000001",
          "00000000-0000-4000-8000-000000000001",
        ],
      }),
    );
    expect(duplicate).toBe(false);
  });

  it("validates complete typed evidence and rejects duplicate resources", () => {
    const review = reviewedVolume();
    expect(
      isValid(
        reviewedPublicationSchema,
        validPublication({ destructiveVolumeReviews: [review] }),
      ),
    ).toBe(true);

    const duplicate = isValid(
      reviewedPublicationSchema,
      validPublication({ destructiveVolumeReviews: [review, review] }),
    );
    expect(duplicate).toBe(false);
    expect(isValid(destructiveVolumeReviewsSchema, [review, review])).toBe(false);
    expect(
      isValid(destructiveVolumeReviewsSchema, [
        reviewedVolume({ usedBytes: Number.POSITIVE_INFINITY }),
      ]),
    ).toBe(false);
    expect(
      isValid(reviewedPublicationSchema, {
        ...validPublication(),
        review: {
          savedStateBasis,
          workingStateFingerprint,
          destructiveServiceIds: ["00000000-0000-4000-8000-000000000001"],
        },
      }),
    ).toBe(false);
    expect(
      isValid(reviewedPublicationSchema, {
        ...validPublication(),
        review: {
          savedStateBasis,
          workingStateFingerprint,
          destructiveVolumeReviews: [review],
        },
      }),
    ).toBe(false);
  });

  it("rejects availability drift even when the identity fingerprint is stable", () => {
    const reviewed = reviewedVolume();
    const fresh = reviewedVolume({ usedBytes: 2048 });

    expect(
      getDestructiveVolumeReviewMismatch({ reviewed: [reviewed], fresh: [fresh] }),
    ).toContain("changed after review");
  });

  it("rejects missing and extra reviewed targets", () => {
    const review = reviewedVolume();
    expect(
      getDestructiveVolumeReviewMismatch({ reviewed: [], fresh: [review] }),
    ).toContain("set of deployed volumes");
    expect(
      getDestructiveVolumeReviewMismatch({ reviewed: [review], fresh: [] }),
    ).toContain("set of deployed volumes");
  });

  it("carries Saved deletion authority until the volume is authored back", () => {
    const review = reviewedVolume();
    expect(
      resolveSavedVolumeDeletionAuthorizations({
        previous: [review],
        fresh: [],
        presentVolumeIds: new Set(),
      }),
    ).toEqual([review]);
    expect(
      resolveSavedVolumeDeletionAuthorizations({
        previous: [review],
        fresh: [],
        presentVolumeIds: new Set([review.target.resourceId]),
      }),
    ).toEqual([]);
  });
});

function reviewedVolume(input?: { usedBytes?: number }): DestructiveVolumeReview {
  return {
    target: {
      version: 1,
      resourceId: "volume-1",
      namespaceId: "production",
      volumeName: "vol-volume-1",
      machineId: "machine-1",
    },
    evidence: {
      version: 1,
      fingerprint: "identity-fingerprint",
      reviewedAt: "2026-07-17T00:00:00.000Z",
      evidence: {
        namespaceId: "production",
        volumeName: "vol-volume-1",
        machineId: "machine-1",
        kind: { kind: "plain" },
        availability: {
          status: "available",
          usedBytes: input?.usedBytes ?? 1024,
          lastWriteUnixSeconds: 1_789_000_000,
        },
        referencingServices: ["api"],
      },
    },
  };
}
