// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ClusterDomainSection } from "./ClusterDomainSection";

describe("ClusterDomainSection", () => {
  afterEach(() => {
    cleanup();
    document.body.replaceChildren();
  });

  it("shows when the records were published and each ingress Server's place in the set", () => {
    render(
      <ClusterDomainSection
        domain={{
          name: "acme.ployz.app",
          recordsSyncedAt: new Date(Date.now() - 5 * 60_000),
          published: [{ machineId: "a", address: "203.0.113.1" }],
          unreachable: [{ machineId: "b", address: "198.51.100.7" }],
        }}
        onPublish={vi.fn()}
      />,
    );

    expect(screen.getByText("Records published 5 minutes ago")).toBeTruthy();
    expect(screen.getAllByRole("listitem").map((item) => item.textContent)).toEqual([
      "203.0.113.1 · in the set",
      "198.51.100.7 · not reachable on port 80",
    ]);
  });

  it("says when nothing is published yet", () => {
    render(
      <ClusterDomainSection
        domain={{ name: "acme.ployz.app", recordsSyncedAt: null, published: [], unreachable: [] }}
        onPublish={vi.fn()}
      />,
    );

    expect(screen.getByText("Records not published yet")).toBeTruthy();
    expect(screen.queryByRole("list")).toBeNull();
  });
});
