// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PublicDomainStatus } from "#/modules/services/public-domain-status";
import { PublicDomainRow } from "./domain-row";

const row = (status: PublicDomainStatus) => (
  <PublicDomainRow
    organizationSlug="acme"
    hostname="www.acme.com"
    title={null}
    portLabel="Port 8080"
    status={status}
    dnsRecords={[{ type: "CNAME", name: "www", value: "acme.ployz.app" }]}
    changed={false}
    onEdit={vi.fn()}
    onDelete={vi.fn()}
  />
);

describe("PublicDomainRow", () => {
  afterEach(() => {
    cleanup();
    document.body.replaceChildren();
  });

  it("opens a live domain and says nothing else", () => {
    render(row({ kind: "live" }));

    expect(screen.getByRole("link").getAttribute("href")).toBe("https://www.acme.com");
    expect(screen.getByText("→ Port 8080")).toBeTruthy();
  });

  it("shows the DNS record to add only when asked", () => {
    render(row({ kind: "needs_dns" }));

    expect(screen.queryByRole("link")).toBeNull();
    expect(screen.queryByText("CNAME")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show DNS records" }));
    expect(screen.getByText("CNAME")).toBeTruthy();
    expect(screen.getByText("acme.ployz.app")).toBeTruthy();
  });
});
