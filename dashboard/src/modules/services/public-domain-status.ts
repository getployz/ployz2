import type { ClusterDomainStatus } from "#/modules/cluster-domain/cluster-domain";
import type { RuntimeCertificateRecord } from "#/modules/runtime/runtime.collection";

/**
 * What a user sees of one public domain on a Service. Generated domains follow the Cluster Domain;
 * custom domains follow their own certificate row, whose refusals name the DNS problem.
 */
export type PublicDomainStatus =
  | { readonly kind: "live" }
  | { readonly kind: "not_deployed" }
  | { readonly kind: "setting_up" }
  | { readonly kind: "issuing" }
  | { readonly kind: "needs_dns" }
  | { readonly kind: "dns_elsewhere" }
  | { readonly kind: "cert_failed"; readonly retryAt: Date | null }
  | { readonly kind: "unreachable" }
  | { readonly kind: "https_down" };

export function publicDomainStatus(input: {
  /** A custom domain (Route) rather than a generated one under the Cluster Domain. */
  readonly custom: boolean;
  /** Whether the hostname is in the Service's Applied State. */
  readonly deployed: boolean;
  /** The hostname's own certificate row; only custom domains read it. */
  readonly certificate: RuntimeCertificateRecord | null;
  /** Null while the Organization holds no Cluster Domain. */
  readonly clusterDomain: ClusterDomainStatus | null;
}): PublicDomainStatus {
  const { clusterDomain, certificate } = input;
  if (!input.deployed) return { kind: "not_deployed" };
  // Every domain lands on the same ingress Servers.
  if (clusterDomain?.kind === "attention" && clusterDomain.reason !== "https_down") return { kind: "unreachable" };
  if (!input.custom) {
    if (clusterDomain?.kind === "attention") return { kind: "https_down" };
    return clusterDomain?.kind === "ready" ? { kind: "live" } : { kind: "setting_up" };
  }
  if (certificate?.status === "available") return { kind: "live" };
  if (certificate?.status !== "failure") return { kind: "issuing" };
  if (certificate.backoff?.failureKind === "does_not_resolve") return { kind: "needs_dns" };
  if (certificate.backoff?.failureKind === "resolves_elsewhere") return { kind: "dns_elsewhere" };
  return { kind: "cert_failed", retryAt: certificate.backoff ? new Date(certificate.backoff.nextAttemptAt) : null };
}

export type DnsRecord = { readonly type: "CNAME" | "A" | "AAAA"; readonly name: string; readonly value: string };

/**
 * The records that point a custom domain here: a CNAME to the Cluster Domain, which follows the
 * ingress Servers, or A/AAAA records to the ingress addresses at an apex or with no Cluster Domain.
 */
export function dnsRecordsFor(hostname: string, clusterDomain: string | null, ingressAddresses: readonly string[]): DnsRecord[] {
  // ponytail: the registrable domain is taken as the last two labels; add a public-suffix list when a
  // multi-label suffix (example.co.uk) shows up.
  const labels = hostname.split(".");
  const name = labels.length > 2 ? labels.slice(0, -2).join(".") : "@";
  if (clusterDomain !== null && name !== "@") return [{ type: "CNAME", name, value: clusterDomain }];
  return ingressAddresses.map((value) => ({ type: value.includes(":") ? "AAAA" : "A", name, value }));
}
