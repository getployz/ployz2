// PROTOTYPE, throwaway. Vercel-style Cluster Domain section: the user sees only what they can act on.
// Open Server Settings with ?variant=A|B|C&state=<key>. Dev builds only.
"use client";

import { useEffect, useState } from "react";
import { ChevronLeftIcon, ChevronRightIcon, LockIcon } from "lucide-react";
import { CopyButton } from "#/components/copy-button";
import { Button } from "#/components/ui/button";
import { Spinner } from "#/components/ui/spinner";

type Status = "none" | "setup" | "ready" | "attention";
type View = {
  status: Status;
  name: string | null;
  /** One sentence. Only when the user should read something. */
  message?: string;
  action?: string;
};
type Reality = { title: string; owner: "user" | "ployz" | "nobody"; view: View };

const NAME = "nick.ployz.app";

// The whole design in one table: what is really happening → what the user sees.
const STATES: Record<string, Reality> = {
  "no-name": { title: "Before the first deploy", owner: "nobody", view: { status: "none", name: null, message: "You’ll get one on your first deploy." } },
  "first-deploy": { title: "First deploy just reserved it", owner: "nobody", view: { status: "setup", name: NAME, message: "This usually takes a few minutes." } },
  healthy: { title: "Healthy", owner: "nobody", view: { status: "ready", name: NAME } },
  degraded: { title: "1 of 2 Servers blocks port 80", owner: "user", view: { status: "attention", name: NAME, message: "One of your servers can’t receive traffic. Make sure port 80 is open on 2a01:4f8::1.", action: "Check again" } },
  "port-80-blocked": { title: "No Server reachable on port 80", owner: "user", view: { status: "attention", name: NAME, message: "Your servers can’t receive traffic. Make sure port 80 is open.", action: "Check again" } },
  "no-public-ip": { title: "No ingress Server with a public IP", owner: "user", view: { status: "attention", name: NAME, message: "None of your servers has a public IP address.", action: "View servers" } },
  "no-servers": { title: "All Servers removed", owner: "user", view: { status: "attention", name: NAME, message: "Add a server to start receiving traffic.", action: "Add server" } },
  "cluster-offline": { title: "Cluster not connected (Servers page says so)", owner: "user", view: { status: "ready", name: NAME } },
  "records-failed": { title: "Hosted DNS down, records stale (auto-retry)", owner: "ployz", view: { status: "setup", name: NAME, message: "Updating. This usually takes a few minutes." } },
  "cert-renewal-failing": { title: "Cert renewal failing, 12 days left (we get alerted)", owner: "ployz", view: { status: "ready", name: NAME } },
  "cert-expired": { title: "Cert expired", owner: "ployz", view: { status: "attention", name: NAME, message: "HTTPS isn’t working right now. We’re fixing it." } },
  "name-rejected": { title: "Hosted DNS rejects our token (we get alerted)", owner: "ployz", view: { status: "ready", name: NAME } },
  "name-replaced": { title: "Name reaped/retired → nick-2", owner: "user", view: { status: "attention", name: "nick-2.ployz.app", message: "Your domain changed from nick.ployz.app. Redeploy to update your services.", action: "Redeploy" } },
};
const STATE_KEYS = Object.keys(STATES);

const VARIANTS = {
  A: { name: "Domain row", Component: VariantA },
  B: { name: "Quiet sentence", Component: VariantB },
  C: { name: "Example URL", Component: VariantC },
} as const;
type VariantKey = keyof typeof VARIANTS;
const VARIANT_KEYS = Object.keys(VARIANTS) as VariantKey[];

export function ClusterDomainPrototype() {
  const [variant, setVariant] = useQueryParam("variant", "A") as [VariantKey, (value: string) => void];
  const [stateKey, setStateKey] = useQueryParam("state", "healthy");
  const [busy, setBusy] = useState(false);
  const view = (STATES[stateKey] ?? STATES["healthy"]!).view;
  // ponytail: stubbed action, 2s spinner then healthy.
  const onAction = () => {
    setBusy(true);
    setTimeout(() => { setBusy(false); setStateKey("healthy"); }, 2000);
  };
  const { Component } = VARIANTS[variant] ?? VARIANTS.A;
  return (
    <>
      <Component view={view} busy={busy} onAction={onAction} />
      <PrototypeBar variant={variant} setVariant={setVariant} stateKey={stateKey} setStateKey={setStateKey} />
    </>
  );
}

type Props = { view: View; busy: boolean; onAction: () => void };

// ── A: Vercel's domains list, one row, status on the right ───────────────────────────────────────
function VariantA({ view, busy, onAction }: Props) {
  return (
    <section className="flex flex-col gap-3">
      <div>
        <h2 className="text-base font-medium">Domain</h2>
        <p className="text-sm text-muted-foreground">Your services get free addresses under this domain.</p>
      </div>
      <div className="rounded-2xl border">
        <div className="flex items-center gap-3 p-4">
          {view.name === null
            ? <span className="text-sm text-muted-foreground">{view.message}</span>
            : <>
                <span className="font-mono text-sm">{view.name}</span>
                <CopyButton value={view.name} label="Copy domain" />
                <span className="ml-auto"><StatusText status={view.status} /></span>
              </>}
        </div>
        {view.name !== null && view.message ? (
          <div className="flex items-center gap-3 border-t px-4 py-3 text-sm text-muted-foreground">
            <span className="flex-1">{view.message}</span>
            {view.action ? <ActionButton label={view.action} busy={busy} onAction={onAction} /> : null}
          </div>
        ) : null}
      </div>
    </section>
  );
}

// ── B: no box at all; a sentence, and only a problem gets a callout ────────────────────────────────
function VariantB({ view, busy, onAction }: Props) {
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-base font-medium">Domain</h2>
      {view.name === null
        ? <p className="text-sm text-muted-foreground">Your services get a free address like <span className="font-mono">web.nick.ployz.app</span>. {view.message}</p>
        : <p className="flex items-center gap-1 text-sm text-muted-foreground">
            Your services get free addresses like <span className="font-mono text-foreground">web.{view.name}</span>
            <CopyButton value={view.name} label="Copy domain" />
          </p>}
      {view.name !== null && view.status !== "ready" && view.message ? (
        <div className={`flex items-center gap-3 rounded-xl border px-4 py-3 text-sm ${view.status === "attention" ? "border-warning-border bg-warning-soft" : ""}`}>
          <StatusDot status={view.status} />
          <span className="flex-1">{view.message}</span>
          {view.action ? <ActionButton label={view.action} busy={busy} onAction={onAction} /> : null}
        </div>
      ) : null}
    </section>
  );
}

// ── C: show a real URL, the way users think about it ───────────────────────────────────────────────
function VariantC({ view, busy, onAction }: Props) {
  const url = `https://web.${view.name ?? "nick.ployz.app"}`;
  return (
    <section className="flex flex-col gap-3 rounded-2xl border p-4">
      <div className="flex items-center justify-between">
        <h2 className="font-medium">Domain</h2>
        {view.name !== null ? <StatusText status={view.status} /> : null}
      </div>
      <div className={`flex items-center gap-2 rounded-lg bg-muted px-3 py-2 font-mono text-sm ${view.name === null ? "opacity-50" : ""}`}>
        <LockIcon className="size-3.5 text-muted-foreground" />
        <span>{url}</span>
        {view.name !== null ? <span className="ml-auto"><CopyButton value={view.name} label="Copy domain" /></span> : null}
      </div>
      <p className="text-sm text-muted-foreground">
        {view.message ?? "Every service you expose gets an address like this, with HTTPS."}
      </p>
      {view.action ? <div><ActionButton label={view.action} busy={busy} onAction={onAction} /></div> : null}
    </section>
  );
}

// ── shared ────────────────────────────────────────────────────────────────────────────────────────
const LABEL: Record<Status, string> = { none: "", setup: "Setting up", ready: "Ready", attention: "Needs attention" };

function StatusDot({ status }: { status: Status }) {
  if (status === "setup") return <Spinner className="size-3.5" />;
  const color = status === "ready" ? "bg-success" : status === "attention" ? "bg-warning" : "bg-muted-foreground";
  return <span className={`size-2 rounded-full ${color}`} />;
}

function StatusText({ status }: { status: Status }) {
  return <span className="flex items-center gap-2 text-sm"><StatusDot status={status} />{LABEL[status]}</span>;
}

function ActionButton({ label, busy, onAction }: { label: string; busy: boolean; onAction: () => void }) {
  return (
    <Button type="button" variant="outline" size="sm" disabled={busy} onClick={onAction}>
      {busy ? <Spinner data-icon="inline-start" /> : null}
      {label}
    </Button>
  );
}

function PrototypeBar({ variant, setVariant, stateKey, setStateKey }: {
  variant: VariantKey;
  setVariant: (value: string) => void;
  stateKey: string;
  setStateKey: (value: string) => void;
}) {
  const cycle = (delta: number) =>
    setVariant(VARIANT_KEYS[(VARIANT_KEYS.indexOf(variant) + delta + VARIANT_KEYS.length) % VARIANT_KEYS.length]!);
  const stepState = (delta: number) =>
    setStateKey(STATE_KEYS[(STATE_KEYS.indexOf(stateKey) + delta + STATE_KEYS.length) % STATE_KEYS.length]!);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.target as HTMLElement).closest("input, textarea, select, [contenteditable]")) return;
      if (event.key === "ArrowLeft") cycle(-1);
      if (event.key === "ArrowRight") cycle(1);
      if (event.key === "ArrowUp") { event.preventDefault(); stepState(-1); }
      if (event.key === "ArrowDown") { event.preventDefault(); stepState(1); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
  const owner = STATES[stateKey]?.owner;
  return (
    <div className="fixed bottom-4 left-1/2 z-50 flex -translate-x-1/2 items-center gap-2 whitespace-nowrap rounded-full bg-neutral-900 px-3 py-2 text-sm text-white shadow-lg">
      <button type="button" aria-label="Previous variant" onClick={() => cycle(-1)}><ChevronLeftIcon className="size-4" /></button>
      <span className="min-w-36 text-center">{variant} ({VARIANTS[variant]?.name})</span>
      <button type="button" aria-label="Next variant" onClick={() => cycle(1)}><ChevronRightIcon className="size-4" /></button>
      <span className="mx-1 h-4 w-px bg-white/30" />
      <span className="text-xs text-white/50">reality:</span>
      <select aria-label="State" className="rounded bg-neutral-800 px-2 py-0.5" value={stateKey} onChange={(event) => setStateKey(event.target.value)}>
        {STATE_KEYS.map((key) => <option key={key} value={key}>{STATES[key]!.title}</option>)}
      </select>
      {owner === "ployz" ? <span className="rounded-full bg-sky-500/20 px-2 text-xs text-sky-300">Ployz fixes · we get alerted</span> : null}
      <span className="text-xs text-white/50">← → layout · ↑ ↓ state</span>
    </div>
  );
}

function useQueryParam(key: string, fallback: string): [string, (value: string) => void] {
  const [value, setValue] = useState(() => new URLSearchParams(window.location.search).get(key) ?? fallback);
  const set = (next: string) => {
    const url = new URL(window.location.href);
    url.searchParams.set(key, next);
    window.history.replaceState(window.history.state, "", url);
    setValue(next);
  };
  return [value, set];
}
