import type {
  DestructiveVolumeReview,
  EnvironmentPublicationSubmissionOutcome,
  ReviewedPublicationInput,
} from "#/modules/deployments/deployment-contract";
import type { EnvironmentSavedStateBasis } from "#/modules/environment-design/saved-state";

export type CanvasPublicationKind = "save" | "deploy";

export function canvasPublicationInput(
  params: {
    organizationSlug: string;
    projectSlug: string;
    environmentSlug: string;
  },
  input: {
    kind: CanvasPublicationKind;
    message: string;
    savedStateBasis: EnvironmentSavedStateBasis;
    reviewedWorkingStateFingerprint: string;
    destructiveServiceIds: string[];
    destructiveVolumeReviews: DestructiveVolumeReview[];
  },
): ReviewedPublicationInput {
  return {
    organizationSlug: params.organizationSlug,
    projectSlug: params.projectSlug,
    environmentSlug: params.environmentSlug,
    intent: input.kind === "deploy" ? "manual_deploy" : "save",
    message: input.message || null,
    review: {
      savedStateBasis: input.savedStateBasis,
      workingStateFingerprint: input.reviewedWorkingStateFingerprint,
      destructiveServiceIds: input.destructiveServiceIds,
      destructiveVolumeReviews: input.destructiveVolumeReviews,
    },
  };
}

export async function submitCanvasPublication(input: {
  submit: (
    data: ReviewedPublicationInput,
  ) => Promise<EnvironmentPublicationSubmissionOutcome>;
  reconcile: () => Promise<void>;
  data: ReviewedPublicationInput;
}): Promise<EnvironmentPublicationSubmissionOutcome> {
  const result = await input.submit(input.data);
  await input.reconcile();
  return result;
}
