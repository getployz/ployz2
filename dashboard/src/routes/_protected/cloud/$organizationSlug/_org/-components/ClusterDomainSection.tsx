"use client";

import { useEffect, useState } from "react";
import { Link } from "@tanstack/react-router";
import { toast } from "sonner";
import { CopyButton } from "#/components/copy-button";
import { Button } from "#/components/ui/button";
import { buttonVariants } from "#/components/ui/button-variants";
import { Item, ItemActions, ItemContent, ItemDescription, ItemTitle } from "#/components/ui/item";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/utils";
import { type ClusterDomainRow, clusterDomainStatus, type ClusterDomainStatus } from "#/modules/cluster-domain/cluster-domain";

/** A check that never lands (a lost event, a failed sync) stops spinning after this. */
const CHECK_TIMEOUT_MS = 60_000;

export function ClusterDomainSection({ organizationSlug, domain, onCheck }: {
  organizationSlug: string;
  domain: Omit<ClusterDomainRow, "id"> | null;
  onCheck: () => Promise<void>;
}) {
  const status = clusterDomainStatus(domain, new Date());
  // The check is running until the row's checkedAt moves past the value it had when clicked.
  const [pendingCheck, setPendingCheck] = useState<{ readonly before: number | null } | null>(null);
  const checkedAt = domain?.checkedAt?.getTime() ?? null;
  const checking = pendingCheck !== null && pendingCheck.before === checkedAt;
  useEffect(() => {
    if (pendingCheck === null) return;
    const timeout = setTimeout(() => setPendingCheck(null), CHECK_TIMEOUT_MS);
    return () => clearTimeout(timeout);
  }, [pendingCheck]);

  function handleCheck() {
    setPendingCheck({ before: checkedAt });
    onCheck().catch((error: unknown) => {
      setPendingCheck(null);
      toast.error(error instanceof Error ? error.message : "The domain couldn’t be checked.");
    });
  }

  return (
    <section aria-labelledby="cluster-domain-heading" className="flex flex-col gap-3">
      <div>
        <h2 id="cluster-domain-heading" className="font-medium">Domain</h2>
        <p className="text-sm text-muted-foreground">Your services get free addresses under this domain.</p>
      </div>
      <Item variant="outline">
        <ItemContent>
          {domain === null
            ? <ItemDescription>You’ll get one on your first deploy.</ItemDescription>
            : <>
                <ItemTitle className="font-mono">{domain.name}</ItemTitle>
                {status.kind === "attention" ? <ItemDescription>{status.message}</ItemDescription> : null}
                {status.kind === "setting_up" ? <ItemDescription>This usually takes a few minutes.</ItemDescription> : null}
              </>}
        </ItemContent>
        {domain === null ? null : (
          <ItemActions>
            <StatusLabel status={status} />
            <CopyButton value={domain.name} label="Copy domain" />
            {status.kind === "attention" && status.action === "check" ? (
              <Button type="button" variant="outline" size="sm" disabled={checking} onClick={handleCheck}>
                {checking ? <Spinner data-icon="inline-start" /> : null}
                Check again
              </Button>
            ) : null}
            {status.kind === "attention" && status.action === "servers" ? (
              <Link
                to="/cloud/$organizationSlug/~/servers"
                params={{ organizationSlug }}
                className={cn(buttonVariants({ variant: "outline", size: "sm" }))}
              >
                View servers
              </Link>
            ) : null}
          </ItemActions>
        )}
      </Item>
    </section>
  );
}

function StatusLabel({ status }: { status: ClusterDomainStatus }) {
  if (status.kind === "none") return null;
  if (status.kind === "setting_up") {
    return <span className="flex items-center gap-2 text-sm"><Spinner />Setting up</span>;
  }
  const ready = status.kind === "ready";
  return (
    <span className="flex items-center gap-2 text-sm">
      <span aria-hidden className={`size-2 rounded-full ${ready ? "bg-success" : "bg-warning"}`} />
      {ready ? "Ready" : "Needs attention"}
    </span>
  );
}
