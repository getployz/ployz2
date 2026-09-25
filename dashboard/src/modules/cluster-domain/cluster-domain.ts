import { Schema } from "effect";
import type { OrganizationClusterDomain } from "#/modules/cluster-domain/tables";

/** The Org Store's view of the Cluster Domain row, keyed by Organization id. Tokens and keys never leave the server. */
export type ClusterDomainRow = { id: string } & Pick<
  OrganizationClusterDomain,
  "name" | "recordsSyncedAt" | "unreachable" | "trafficIssue" | "certificateNotAfter" | "checkedAt"
>;

export const CheckClusterDomainInput = Schema.Struct({
  organizationSlug: Schema.String.check(Schema.isTrimmed(), Schema.isNonEmpty()),
});

/**
 * What the user sees of the Cluster Domain. Only problems the user can fix, or that break their
 * traffic, surface; the rest are Ployz's to fix and read as ready or setting up.
 */
export type ClusterDomainStatus =
  | { readonly kind: "none" }
  | { readonly kind: "setting_up" }
  | { readonly kind: "ready" }
  | { readonly kind: "attention"; readonly message: string; readonly action: "check" | "servers" | null };

export function clusterDomainStatus(
  domain: Pick<ClusterDomainRow, "recordsSyncedAt" | "unreachable" | "trafficIssue" | "certificateNotAfter"> | null,
  now: Date,
): ClusterDomainStatus {
  if (domain === null) return { kind: "none" };
  if (domain.trafficIssue === "no_servers") {
    return { kind: "attention", message: "Add a server to start receiving traffic.", action: "servers" };
  }
  if (domain.trafficIssue === "no_public_ip") {
    return { kind: "attention", message: "None of your servers has a public IP address.", action: "servers" };
  }
  if (domain.unreachable.length > 0) {
    const addresses = domain.unreachable.map((server) => server.address).join(", ");
    return { kind: "attention", message: `Traffic can’t reach ${addresses}. Make sure port 80 is open.`, action: "check" };
  }
  if (domain.certificateNotAfter !== null && domain.certificateNotAfter <= now) {
    return { kind: "attention", message: "HTTPS isn’t working right now. We’re fixing it.", action: null };
  }
  if (domain.recordsSyncedAt === null || domain.certificateNotAfter === null) return { kind: "setting_up" };
  return { kind: "ready" };
}
