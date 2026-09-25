import { describe, expect, it } from "vitest";
import type { ClusterDomainStatus } from "#/modules/cluster-domain/cluster-domain";
import type { RuntimeCertificateRecord } from "#/modules/runtime/runtime.collection";
import { dnsRecordsFor, publicDomainStatus } from "#/modules/services/public-domain-status";

const ready: ClusterDomainStatus = { kind: "ready" };
const certificate = (status: string, failureKind?: string): RuntimeCertificateRecord => ({
  hostname: "www.acme.com",
  status,
  lastError: failureKind ? "raw ACME text" : null,
  backoff: failureKind ? { failureKind, nextAttemptAt: "2026-09-25T12:12:00Z", failures: 2 } : null,
});

describe("publicDomainStatus", () => {
  it.each([
    ["an undeployed domain waits for the next deploy",
      { custom: true, deployed: false, certificate: certificate("available"), clusterDomain: ready }, { kind: "not_deployed" }],
    ["servers that can't receive traffic affect every domain",
      { custom: true, deployed: true, certificate: certificate("available"), clusterDomain: { kind: "attention", reason: "port_80", addresses: ["203.0.113.1"] } },
      { kind: "unreachable" }],
    ["no servers affect every domain",
      { custom: false, deployed: true, certificate: null, clusterDomain: { kind: "attention", reason: "no_servers" } }, { kind: "unreachable" }],
    ["a generated domain is live once the Cluster Domain is ready",
      { custom: false, deployed: true, certificate: null, clusterDomain: ready }, { kind: "live" }],
    ["a generated domain sets up with its Cluster Domain",
      { custom: false, deployed: true, certificate: null, clusterDomain: { kind: "setting_up" } }, { kind: "setting_up" }],
    ["a generated domain goes down with an expired wildcard",
      { custom: false, deployed: true, certificate: null, clusterDomain: { kind: "attention", reason: "https_down" } }, { kind: "https_down" }],
    ["an expired wildcard leaves custom domains alone",
      { custom: true, deployed: true, certificate: certificate("available"), clusterDomain: { kind: "attention", reason: "https_down" } }, { kind: "live" }],
    ["a custom domain with a certificate is live",
      { custom: true, deployed: true, certificate: certificate("available"), clusterDomain: null }, { kind: "live" }],
    ["a custom domain that doesn't resolve needs DNS",
      { custom: true, deployed: true, certificate: certificate("failure", "does_not_resolve"), clusterDomain: ready }, { kind: "needs_dns" }],
    ["a custom domain resolving elsewhere needs DNS pointed here",
      { custom: true, deployed: true, certificate: certificate("failure", "resolves_elsewhere"), clusterDomain: ready }, { kind: "dns_elsewhere" }],
    ["a refused certificate retries at the next attempt",
      { custom: true, deployed: true, certificate: certificate("failure", "authority"), clusterDomain: ready },
      { kind: "cert_failed", retryAt: new Date("2026-09-25T12:12:00Z") }],
    ["a pending certificate is issuing",
      { custom: true, deployed: true, certificate: certificate("pending"), clusterDomain: ready }, { kind: "issuing" }],
    ["no certificate row yet is issuing",
      { custom: true, deployed: true, certificate: null, clusterDomain: null }, { kind: "issuing" }],
  ] as const)("%s", (_, input, expected) => {
    expect(publicDomainStatus(input)).toEqual(expected);
  });
});

describe("dnsRecordsFor", () => {
  const ips = ["203.0.113.1", "2001:db8::1"];

  it("points a subdomain at the Cluster Domain", () => {
    expect(dnsRecordsFor("www.acme.com", "nick.ployz.app", ips)).toEqual([{ type: "CNAME", name: "www", value: "nick.ployz.app" }]);
    expect(dnsRecordsFor("a.b.acme.com", "nick.ployz.app", ips)).toEqual([{ type: "CNAME", name: "a.b", value: "nick.ployz.app" }]);
  });

  it("points an apex at the ingress addresses, since a CNAME can't sit there", () => {
    expect(dnsRecordsFor("acme.com", "nick.ployz.app", ips)).toEqual([
      { type: "A", name: "@", value: "203.0.113.1" },
      { type: "AAAA", name: "@", value: "2001:db8::1" },
    ]);
  });

  it("points at the ingress addresses when there is no Cluster Domain", () => {
    expect(dnsRecordsFor("www.acme.com", null, ["203.0.113.1"])).toEqual([{ type: "A", name: "www", value: "203.0.113.1" }]);
  });
});
