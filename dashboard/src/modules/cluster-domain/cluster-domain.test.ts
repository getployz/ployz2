import { describe, expect, it } from "vitest";
import { type ClusterDomainRow, clusterDomainStatus } from "#/modules/cluster-domain/cluster-domain";

const now = new Date("2026-09-25T12:00:00Z");
const ready: Pick<ClusterDomainRow, "recordsSyncedAt" | "unreachable" | "trafficIssue" | "certificateNotAfter"> = {
  recordsSyncedAt: new Date("2026-09-25T11:00:00Z"),
  unreachable: [],
  trafficIssue: null,
  certificateNotAfter: new Date("2026-12-25T00:00:00Z"),
};
const blocked = [{ machineId: "a", address: "203.0.113.1" }, { machineId: "b", address: "2001:db8::1" }] as ClusterDomainRow["unreachable"];

describe("clusterDomainStatus", () => {
  it.each([
    ["no name", null, { kind: "none" }],
    ["healthy", ready, { kind: "ready" }],
    ["records not yet published", { ...ready, recordsSyncedAt: null }, { kind: "setting_up" }],
    ["no wildcard yet", { ...ready, certificateNotAfter: null }, { kind: "setting_up" }],
    ["no Servers", { ...ready, trafficIssue: "no_servers" },
      { kind: "attention", message: "Add a server to start receiving traffic.", action: "servers" }],
    ["no public IP", { ...ready, trafficIssue: "no_public_ip" },
      { kind: "attention", message: "None of your servers has a public IP address.", action: "servers" }],
    ["port 80 blocked", { ...ready, unreachable: blocked },
      { kind: "attention", message: "Traffic can’t reach 203.0.113.1, 2001:db8::1. Make sure port 80 is open.", action: "check" }],
    ["wildcard expired", { ...ready, certificateNotAfter: new Date("2026-09-24T00:00:00Z") },
      { kind: "attention", message: "HTTPS isn’t working right now. We’re fixing it.", action: null }],
    // What the user can fix comes before what Ployz is fixing or still setting up.
    ["no Servers and nothing published", { ...ready, trafficIssue: "no_servers", recordsSyncedAt: null },
      { kind: "attention", message: "Add a server to start receiving traffic.", action: "servers" }],
  ] as const)("%s", (_, domain, expected) => {
    expect(clusterDomainStatus(domain, now)).toEqual(expected);
  });
});
