import { toast } from "sonner";
import { authClient } from "./auth-client";

/**
 * The per-user "open deployments I start" preference, stored on the Better Auth user.
 * Read at call time from the session store, which Better Auth refetches after `update-user`.
 */
export function openStartedDeployments() {
  return authClient.$store.atoms["session"]?.get().data?.user.openStartedDeployments ?? true;
}

export async function setOpenStartedDeployments(value: boolean) {
  if (openStartedDeployments() === value) return;
  const result = await authClient.updateUser({ openStartedDeployments: value });
  if (result.error) toast.error("Couldn't save your deployment preference.");
}
