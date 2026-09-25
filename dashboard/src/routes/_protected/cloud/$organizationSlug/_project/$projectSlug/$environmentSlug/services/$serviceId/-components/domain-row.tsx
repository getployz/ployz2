import { type ReactNode, useState } from "react";
import { Link } from "@tanstack/react-router";
import { AlertTriangleIcon, ArrowUpRightIcon, GlobeIcon, PencilIcon, Trash2Icon } from "lucide-react";
import { CopyButton } from "#/components/copy-button";
import { Button } from "#/components/ui/button";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/utils";
import type { DnsRecord, PublicDomainStatus } from "#/modules/services/public-domain-status";
import { formatRelativeTime } from "#/utils/relative-time";

export function DomainTitle({
  hostname,
  copyLabel,
  href,
}: {
  hostname: string;
  copyLabel: string;
  /** Set when the domain is live, so the name opens it. */
  href?: string;
}) {
  return (
    <div className="flex min-w-0 items-center gap-1">
      {href ? (
        <a href={href} target="_blank" rel="noreferrer" className="flex min-w-0 items-center gap-1 font-mono text-sm hover:underline">
          <span className="truncate">{hostname}</span>
          <ArrowUpRightIcon className="size-3.5 shrink-0 text-muted-foreground" />
        </a>
      ) : (
        <span className="truncate font-mono text-sm">{hostname}</span>
      )}
      <CopyButton value={hostname} label={copyLabel} size="icon-xs" />
    </div>
  );
}

export function DomainRowShell({
  icon,
  children,
  changed,
  actions,
}: {
  icon: ReactNode;
  children: ReactNode;
  changed?: boolean;
  actions: ReactNode;
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-3 rounded-lg border bg-card p-3",
        changed && "border-changed-border bg-changed-soft"
      )}
    >
      <span className="text-muted-foreground">{icon}</span>
      <div className="min-w-0 flex-1">{children}</div>
      <div className="flex items-center gap-1">{actions}</div>
    </div>
  );
}

function StatusIcon({ status }: { status: PublicDomainStatus }) {
  switch (status.kind) {
    case "live":
      return <GlobeIcon />;
    case "not_deployed":
      return <GlobeIcon className="opacity-50" />;
    case "setting_up":
    case "issuing":
      return <Spinner />;
    default:
      return <AlertTriangleIcon className="text-warning" />;
  }
}

/** One short phrase per status; the link, if any, is the one next step. */
function statusPhrase(status: PublicDomainStatus): string | null {
  switch (status.kind) {
    case "live":
      return null;
    case "not_deployed":
      return "Live after your next deploy";
    case "setting_up":
      return "Setting up";
    case "issuing":
      return "Issuing certificate";
    case "needs_dns":
      return "Waiting for DNS update";
    case "dns_elsewhere":
      return "DNS points somewhere else";
    case "cert_failed":
      return status.retryAt ? `Certificate failed · retrying ${formatRelativeTime(status.retryAt)}` : "Certificate failed";
    case "unreachable":
      return "Servers can’t receive traffic";
    case "https_down":
      return "HTTPS is down · we’re fixing it";
  }
}

function DnsRecords({ records }: { records: DnsRecord[] }) {
  return (
    <div className="grid grid-cols-[auto_auto_1fr] items-center gap-x-6 gap-y-1 rounded-md bg-muted px-3 py-2 font-mono text-xs">
      <span className="text-muted-foreground">Type</span>
      <span className="text-muted-foreground">Name</span>
      <span className="text-muted-foreground">Value</span>
      {records.map((record) => (
        <div key={`${record.type}-${record.value}`} className="contents">
          <span>{record.type}</span>
          <span>{record.name}</span>
          <span className="flex min-w-0 items-center gap-1">
            <span className="truncate">{record.value}</span>
            <CopyButton value={record.value} label={`Copy ${record.type} value`} size="icon-xs" />
          </span>
        </div>
      ))}
    </div>
  );
}

/** A public domain on a Service: the icon is its status, and one line says what's next. */
export function PublicDomainRow({
  organizationSlug,
  title,
  hostname,
  portLabel,
  status,
  dnsRecords,
  changed,
  onEdit,
  onDelete,
}: {
  organizationSlug: string;
  /** The hostname, or a placeholder while it has none. */
  title: ReactNode;
  hostname: string | null;
  portLabel: string;
  status: PublicDomainStatus;
  /** The records that point the domain here; only custom domains have them. */
  dnsRecords: DnsRecord[];
  changed: boolean;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const [showDns, setShowDns] = useState(false);
  const phrase = statusPhrase(status);
  const needsDns = (status.kind === "needs_dns" || status.kind === "dns_elsewhere") && dnsRecords.length > 0;
  return (
    <div className="flex flex-col gap-2">
      <DomainRowShell
        changed={changed}
        icon={<StatusIcon status={status} />}
        actions={
          <>
            <Button type="button" variant="ghost" size="icon-sm" aria-label={`Edit ${hostname ?? "domain"}`} onClick={onEdit}>
              <PencilIcon />
            </Button>
            <Button type="button" variant="ghost" size="icon-sm" aria-label={`Remove ${hostname ?? "domain"}`} onClick={onDelete}>
              <Trash2Icon />
            </Button>
          </>
        }
      >
        {hostname ? (
          <DomainTitle hostname={hostname} copyLabel={`Copy ${hostname}`} href={status.kind === "live" ? `https://${hostname}` : undefined} />
        ) : (
          title
        )}
        <div className="text-muted-foreground text-sm">
          → {portLabel}
          {phrase ? ` · ${phrase}` : null}
          {needsDns ? (
            <>
              {" · "}
              <button type="button" className="text-foreground underline-offset-4 hover:underline" onClick={() => setShowDns(!showDns)}>
                {showDns ? "Hide DNS records" : "Show DNS records"}
              </button>
            </>
          ) : null}
          {status.kind === "unreachable" ? (
            <>
              {" · "}
              <Link to="/cloud/$organizationSlug/~/settings" params={{ organizationSlug }} className="text-foreground underline-offset-4 hover:underline">
                Server Settings
              </Link>
            </>
          ) : null}
        </div>
      </DomainRowShell>
      {needsDns && showDns ? <DnsRecords records={dnsRecords} /> : null}
    </div>
  );
}
