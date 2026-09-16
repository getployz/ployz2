import { useState } from "react";
import { reconcileDeploymentCollections } from "#/modules/deployments/deployment-collection";
import { useCollectionScope } from "#/collections/use-collection-scope";
import { useEnvironmentDocument } from "#/modules/environment-design/environment-document.collection";
import { discardEnvironmentChangesServerFn } from "#/modules/environment-design/working-document-restore.functions";
import type { DiscardEnvironmentChangesInput } from "#/modules/environment-design/working-document-restore";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useServerFn } from "@tanstack/react-start";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import {
  getEnvironmentsCollection,
} from "#/collections/collections";
import type {
  CanvasEnvironmentChangeGroup,
  CanvasEnvironmentChangeState,
} from "#/modules/environment-design/canvas-environment-change-state";
import type {
  DestructiveVolumeReview,
} from "#/modules/deployments/deployment-contract";
import type {
  EnvironmentSavedStateBasis,
} from "#/modules/environment-design/saved-state";
import { getDeployTargetPreflight } from "#/modules/runtime/deploy-target-preflight";
import { useRuntimeLens } from "#/modules/runtime/use-runtime-lens";
import {
  prepareEnvironmentDestructiveVolumesServerFn,
  submitReviewedPublicationServerFn,
} from "#/modules/deployments/deployment.functions";
import { serviceDeploymentKeys } from "#/modules/deployments/deployment-queries";
import type { PreparedDestructiveReview } from "#/components/destructive-volume/volume-destruction-confirmation-dialog";
import { prepareVolumeDestructionReview } from "#/components/destructive-volume/destructive-volume-review";
import {
  fingerprintReviewedEnvironmentWorkingState,
  projectReviewedEnvironmentWorkingState,
} from "#/modules/environment-design/working-state-review";
import {
  canvasPublicationInput,
  submitCanvasPublication,
  type CanvasPublicationKind,
} from "./canvas-publication-submission";

type EnvironmentRouteParams = {
  organizationSlug: string;
  projectSlug: string;
  environmentSlug: string;
};

type UseCanvasChangeActionsInput = {
  environmentId: string;
  params: EnvironmentRouteParams;
  changeState: CanvasEnvironmentChangeState;
  savedSnapshotSource: {
    kind: "saved";
    environmentSavedStateSnapshotId: string;
  } | null;
  destructiveServiceIds: string[];
  deletedDeployedVolumeIds: string[];
  commitMessage: string;
  setCommitMessage: (message: string) => void;
  setDestructiveConfirmationOpen: (open: boolean) => void;
};

export function useCanvasChangeActions({
  environmentId,
  params,
  changeState,
  savedSnapshotSource,
  destructiveServiceIds,
  deletedDeployedVolumeIds,
  commitMessage,
  setCommitMessage,
  setDestructiveConfirmationOpen,
}: UseCanvasChangeActionsInput) {
  const [pendingPublication, setPendingPublication] =
    useState<CanvasPublicationKind>("save");
  const collectionScope = useCollectionScope();
  const document = useEnvironmentDocument(params.organizationSlug, environmentId);
  function workingReview() {
    if (!document) throw new Error("Environment is not loaded.");
    return projectReviewedEnvironmentWorkingState(document);
  }
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const environments = getEnvironmentsCollection(params.organizationSlug, collectionScope);
  const runtime = useRuntimeLens(params.organizationSlug);
  const deployTargetPreflight = getDeployTargetPreflight({
    status: runtime.status,
    machineCount: runtime.machines.length,
    error: runtime.error,
    isLoading: runtime.isLoading,
  });
  const savedStateBasis: EnvironmentSavedStateBasis = savedSnapshotSource
    ? {
        kind: "saved_revision",
        savedStateSnapshotId:
          savedSnapshotSource.environmentSavedStateSnapshotId,
      }
    : { kind: "no_saved_state" };
  const submitReviewedPublication = useServerFn(
    submitReviewedPublicationServerFn,
  );
  const discard = useServerFn(discardEnvironmentChangesServerFn);
  const prepareEnvironmentDestructiveVolumes = useServerFn(
    prepareEnvironmentDestructiveVolumesServerFn,
  );
  const publicationMutation = useMutation({
    mutationFn: async (input: {
      kind: CanvasPublicationKind;
      message: string;
      savedStateBasis: EnvironmentSavedStateBasis;
      destructiveServiceIds: string[];
      destructiveVolumeReviews: DestructiveVolumeReview[];
      reviewedWorkingStateFingerprint?: string;
    }) => {
      const reviewedWorkingStateFingerprint =
        input.reviewedWorkingStateFingerprint ??
        (await fingerprintReviewedEnvironmentWorkingState(
          workingReview(),
        ));
      return submitCanvasPublication({
        submit: (data) => submitReviewedPublication({ data }),
        reconcile: async () => {
          await reconcileDeploymentCollections(
            params.organizationSlug,
            collectionScope,
          );
          await queryClient.invalidateQueries({
            queryKey: serviceDeploymentKeys.environmentChangeStatesOrg(
              params.organizationSlug,
            ),
          });
        },
        data: canvasPublicationInput(params, {
          kind: input.kind,
          message: input.message,
          savedStateBasis: input.savedStateBasis,
          reviewedWorkingStateFingerprint,
          destructiveServiceIds: input.destructiveServiceIds,
          destructiveVolumeReviews: input.destructiveVolumeReviews,
        }),
      });
    },
  });

  async function discardChanges(command: DiscardEnvironmentChangesInput["command"]) {
    try {
      if (!document) throw new Error("Environment is not loaded.");
      const result = await discard({ data: {
        organizationSlug: params.organizationSlug, environmentId, revision: document.revision,
        savedStateBasis, baselineToken: changeState.baselineToken, command,
      } });
      await environments.writeCommitted(result.data);
      await reconcileDeploymentCollections(params.organizationSlug, collectionScope);
      await queryClient.invalidateQueries({
        queryKey: serviceDeploymentKeys.environmentChangeStatesOrg(params.organizationSlug),
      });
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Could not discard changes.");
    }
  }

  function discardAllChanges() {
    return discardChanges({ kind: "all" });
  }

  function discardNodeChanges(group: CanvasEnvironmentChangeGroup) {
    return discardChanges({ kind: "node", nodeType: group.nodeType, nodeId: group.nodeId });
  }

  function discardRowChange(group: CanvasEnvironmentChangeGroup, path: string) {
    return discardChanges({ kind: "node", nodeType: group.nodeType, nodeId: group.nodeId, path });
  }

  function deployTargetIsAvailable() {
    if (!deployTargetPreflight.ok) {
      toast.error(deployTargetPreflight.title, {
        description: deployTargetPreflight.description,
        action:
          deployTargetPreflight.action === "add_server"
            ? {
                label: "Add machine",
                onClick: () => {
                  void navigate({
                    to: "/cloud/$organizationSlug/~/servers",
                    params: { organizationSlug: params.organizationSlug },
                  });
                },
              }
            : undefined,
      });
      return false;
    }
    return true;
  }

  function hasDestructiveChanges() {
    return (
      destructiveServiceIds.length > 0 || deletedDeployedVolumeIds.length > 0
    );
  }

  async function submitPublication(
    kind: CanvasPublicationKind,
    review: {
      destructiveServiceIds: string[];
      destructiveVolumeReviews: DestructiveVolumeReview[];
      reviewedWorkingStateFingerprint?: string;
    },
  ) {
    const outcome = await publicationMutation.mutateAsync({
      kind,
      message: commitMessage,
      savedStateBasis,
      destructiveServiceIds: review.destructiveServiceIds,
      destructiveVolumeReviews: review.destructiveVolumeReviews,
      reviewedWorkingStateFingerprint: review.reviewedWorkingStateFingerprint,
    });
    if (outcome.state === "attempt_dispatch_failed") {
      toast.error("Cloud could not dispatch the deployment workflow.");
      setCommitMessage("");
      return outcome;
    }
    if (outcome.state === "deployment_queued" || outcome.state === "saved") {
      setCommitMessage("");
    }
    return outcome;
  }

  async function requestPublication(kind: CanvasPublicationKind) {
    if (kind === "deploy" && !deployTargetIsAvailable()) return;
    if (hasDestructiveChanges()) {
      setPendingPublication(kind);
      setDestructiveConfirmationOpen(true);
      return;
    }
    try {
      await submitPublication(kind, {
        destructiveServiceIds: [],
        destructiveVolumeReviews: [],
      });
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : `Failed to ${kind} the Working State.`,
      );
    }
  }

  function requestDeploy() {
    return requestPublication("deploy");
  }

  function requestSave() {
    return requestPublication("save");
  }

  async function prepareDestructiveReview() {
    const reviewedMutation = {
      savedStateBasis,
      workingStateFingerprint:
        await fingerprintReviewedEnvironmentWorkingState(
          workingReview(),
        ),
      serviceIds: [...destructiveServiceIds],
      volumeIds: [...deletedDeployedVolumeIds],
    };
    const reviews = await prepareEnvironmentDestructiveVolumes({
        data: {
          organizationSlug: params.organizationSlug,
          projectSlug: params.projectSlug,
          environmentSlug: params.environmentSlug,
        },
      });
    return {
      ...prepareVolumeDestructionReview({
        reviews,
        expectedResourceIds: reviewedMutation.volumeIds,
        expectedNamespaceId: params.environmentSlug,
      }),
      reviewedMutation,
    };
  }

  async function confirmDestructiveAction(
    preparation: PreparedDestructiveReview,
  ) {
    if (!preparation.reviewedMutation) {
      throw new Error(
        "The destructive publication is missing its reviewed mutation.",
      );
    }
    const reviewedMutation = preparation.reviewedMutation;
    const outcome = await submitPublication(pendingPublication, {
      destructiveServiceIds: reviewedMutation.serviceIds,
      destructiveVolumeReviews: preparation.reviews,
      reviewedWorkingStateFingerprint:
        reviewedMutation.workingStateFingerprint,
    });
    if (outcome.state === "review_updated_evidence") {
      return {
        state: "review_updated_evidence" as const,
        preparation: {
          ...prepareVolumeDestructionReview({
            reviews: outcome.freshReviews,
            expectedResourceIds: reviewedMutation.volumeIds,
            expectedNamespaceId: params.environmentSlug,
          }),
          reviewedMutation,
        },
      };
    }
    return { state: "submitted" as const };
  }

  return {
    discardAllChanges,
    discardNodeChanges,
    discardRowChange,
    requestSave,
    requestDeploy,
    pendingPublication,
    prepareDestructiveReview,
    confirmDestructiveAction,
  };
}
