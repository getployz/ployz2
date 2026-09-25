import type { PloyzInngest } from "#/modules/inngest/client";
import type { PolarConfiguration } from "#/server/config.server";
import {
  createScheduleNightlyBillingReconcile,
  createSyncOrganizationBillingStateFunction,
} from "#/modules/billing/inngest-sync/sync";
import {
  createProcessGithubCheckSuiteReceived,
  createProcessGithubPushReceived,
} from "#/modules/github/inngest-ingestion/process";
import { createSweepGithubIngestionOutboxes } from "#/modules/github/inngest-ingestion/sweep";
import { createScheduleGithubRepositorySync } from "#/modules/github/inngest-sync/scheduled";
import { createSyncGithubRepositories } from "#/modules/github/inngest-sync/sync";
import {
  createProcessGithubInstallationReceived,
  createProcessGithubInstallationRepositoriesReceived,
} from "#/modules/github/inngest-sync/webhook";
import {
  createMarkCancelledRowBackedWorkflow,
  createProcessEnvironmentDeployment,
} from "#/modules/deployments/environment-deployment.inngest";
import {
  createCancelMachineRemove,
  createProcessMachineRemove,
} from "#/modules/machines/machine-removal.inngest";
import {
  createCancelTeardown,
  createProcessTeardown,
} from "#/modules/runtime/teardown.inngest";
import { createPruneOrganizationChangeLog } from "#/modules/organization/change-log.inngest";
import {
  createCancelVolumeRemove,
  createProcessVolumeRemove,
} from "#/modules/runtime/volume-removal.inngest";

/** A Self-hosted Cloud has no Polar, so billing sync is never registered. */
export function createInngestFunctions(
  inngest: PloyzInngest,
  billingMode: PolarConfiguration["mode"],
) {
  const billing = billingMode === "hosted"
    ? [
        createSyncOrganizationBillingStateFunction(inngest),
        createScheduleNightlyBillingReconcile(inngest),
      ]
    : [];
  return [
    createProcessGithubInstallationReceived(inngest),
    createProcessGithubInstallationRepositoriesReceived(inngest),
    createProcessGithubPushReceived(inngest),
    createProcessGithubCheckSuiteReceived(inngest),
    createSweepGithubIngestionOutboxes(inngest),
    createSyncGithubRepositories(inngest),
    createMarkCancelledRowBackedWorkflow(inngest),
    createProcessEnvironmentDeployment(inngest),
    createScheduleGithubRepositorySync(inngest),
    ...billing,
    createProcessMachineRemove(inngest),
    createCancelMachineRemove(inngest),
    createProcessVolumeRemove(inngest),
    createCancelVolumeRemove(inngest),
    createProcessTeardown(inngest),
    createCancelTeardown(inngest),
    createPruneOrganizationChangeLog(inngest),
  ];
}
