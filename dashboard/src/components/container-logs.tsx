import { useLayoutEffect, useRef, useState, useId } from "react";
import { useLiveQuery } from "@tanstack/react-db";
import { useMutation } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Schema } from "effect";
import { useCollectionScope } from "#/collections/use-collection-scope";
import { createContainerLogs, appendContainerLogs, mergeContainerHistory, remainingHistory, containerLogEventSchema, containerLogPageSchema, type ContainerLogRow, type ContainerLogs } from "#/modules/runtime/container-log.collection";
import { Button } from "#/components/ui/button";
import { Input } from "#/components/ui/input";
import { Select, SelectTrigger, SelectValue, SelectContent, SelectGroup, SelectItem } from "#/components/ui/select";

export type ContainerLogSelection = { organizationSlug: string; environmentSlug?: string; deploymentId?: string; serviceId?: string };

export function ContainerLogs({ selection, lifecycle = [] }: { selection: ContainerLogSelection; lifecycle?: readonly ContainerLogRow[] }) {
  const scope = useCollectionScope();
  const [generation, setGeneration] = useState(0);
  const key = JSON.stringify([scope.sessionId, scope.userId, selection, generation]);
  return <LogViewer key={key} selection={selection} lifecycle={lifecycle} refresh={() => setGeneration(value => value + 1)} />;
}

function LogViewer({ selection, lifecycle, refresh }: { selection: ContainerLogSelection; lifecycle: readonly ContainerLogRow[]; refresh: () => void }) {
  const scope = useCollectionScope();
  const id = useId();
  const historyAbort = useRef(new AbortController());
  const connection = useRef<() => () => void>(() => () => {});
  const [collection] = useState(() => createContainerLogs(`container-logs:${scope.sessionId}:${id}`, () => connection.current()));
  const [status, setStatus] = useState("Connecting…");
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [search, setSearch] = useState("");
  const [machine, setMachine] = useState("");
  const [service, setService] = useState("");
  const [exhausted, setExhausted] = useState<Record<string, string>>({});
  const element = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const anchor = useRef<{ id: string; offset: number } | null>(null);
  const query = new URLSearchParams(Object.entries(selection).filter((entry): entry is [string, string] => entry[1] !== undefined)).toString();
  connection.current = () => {
    const stop = openContainerLogStream(collection, query, setStatus, setErrors);
    return () => { stop(); historyAbort.current.abort(); };
  };
  const { data: loaded = [] } = useLiveQuery({ queryKey: ["container-logs", collection.id], query: q => q.from({ log: collection }) });
  const rows = [...lifecycle, ...loaded].filter(row => (!machine || row.machineId === machine) && (!service || row.serviceName === service) && row.message.toLowerCase().includes(search.toLowerCase())).sort((a, b) => {
    const difference = BigInt(a.timestamp) - BigInt(b.timestamp);
    return difference < 0n ? -1 : difference > 0n ? 1 : a.id.localeCompare(b.id);
  });
  const virtual = useVirtualizer({ count: rows.length, getScrollElement: () => element.current, estimateSize: () => 24, getItemKey: index => rows[index]?.id ?? index, overscan: 12 });
  const history = useMutation({
    mutationFn: async () => {
      const before = remainingHistory(loaded, exhausted);
      const response = await fetch(`/api/runtime/logs?${query}&before=${encodeURIComponent(JSON.stringify(before))}`, { signal: historyAbort.current.signal });
      if (!response.ok) throw new Error("Could not load older logs.");
      const page = Schema.decodeUnknownSync(containerLogPageSchema)(await response.json());
      return { page, before };
    },
    onSuccess: ({ page, before }) => {
      const first = virtual.getVirtualItems().find(item => item.end > (element.current?.scrollTop ?? 0));
      const firstRow = first && rows[first.index];
      anchor.current = first && firstRow ? { id: firstRow.id, offset: (element.current?.scrollTop ?? 0) - first.start } : null;
      following.current = false;
      mergeContainerHistory(collection, page.records);
      setErrors(Object.fromEntries(page.errors.map(error => [`${error.machineId}/${error.containerId}`, error.message])));
      setExhausted(previous => {
        const next = { ...previous };
        for (const [source, boundary] of Object.entries(before)) {
          const failed = page.errors.some(error => `${error.machineId}/${error.containerId}` === source);
          const progressed = page.records.some(row => `${row.machineId}/${row.containerId}` === source && BigInt(row.timestamp) < BigInt(boundary));
          if (!failed && !progressed) next[source] = boundary;
        }
        return next;
      });
    },
  });
  useLayoutEffect(() => {
    if (anchor.current) {
      const index = rows.findIndex(row => row.id === anchor.current?.id);
      if (index >= 0) {
        virtual.scrollToIndex(index, { align: "start" });
        if (element.current) element.current.scrollTop += anchor.current.offset;
      }
      anchor.current = null;
    } else if (following.current && rows.length) virtual.scrollToIndex(rows.length - 1, { align: "end" });
  }, [rows, virtual]);
  const machines = new Map(loaded.map(row => [row.machineId, row.machineName]));
  const services = [...new Set(loaded.map(row => row.serviceName))];
  return <div className="flex min-h-0 flex-col gap-3">
    <div className="flex flex-wrap items-center gap-2">
      <Input aria-label="Search loaded logs" placeholder="Search loaded logs" value={search} onChange={event => setSearch(event.target.value)} className="min-w-40 flex-1" />
      <LogFilter label="All services" value={service} onChange={setService} options={services.map(name => [name, name])} />
      <LogFilter label="All servers" value={machine} onChange={setMachine} options={[...machines]} />
      <Button variant="ghost" size="sm" onClick={refresh}>Refresh</Button>
    </div>
    <div className="flex items-center justify-between gap-2">
      <Button variant="outline" size="sm" disabled={history.isPending || !Object.keys(remainingHistory(loaded, exhausted)).length} onClick={() => history.mutate()}>{history.isPending ? "Loading…" : "Load older"}</Button>
      <span role="status" className="text-xs text-muted-foreground">{status}</span>
      <Button variant="ghost" size="sm" onClick={() => { following.current = true; if (rows.length) virtual.scrollToIndex(rows.length - 1, { align: "end" }); }}>Latest</Button>
    </div>
    {history.error ? <p role="alert">Could not load older logs.</p> : null}
    {Object.entries(errors).map(([source, message]) => <p role="alert" key={source}>{source}: {message}</p>)}
    <div ref={element} tabIndex={0} aria-label="Container logs" className="h-80 overflow-auto font-mono text-xs" onScroll={() => { const node = element.current; if (node) following.current = node.scrollHeight - node.scrollTop - node.clientHeight < 48; }}>
      {!rows.length ? <p className="text-muted-foreground">No matching output available.</p> : null}
      <div className="relative w-full" style={{ height: virtual.getTotalSize() }}>
        {virtual.getVirtualItems().map(item => {
          const row = rows[item.index];
          if (!row) return null;
          return <div key={item.key} ref={virtual.measureElement} data-index={item.index} className="absolute left-0 top-0 w-full whitespace-pre-wrap break-words leading-6" style={{ transform: `translateY(${item.start}px)` }}>
            <time className="mr-3 text-muted-foreground">{new Date(Number(BigInt(row.timestamp) / 1_000_000n)).toISOString().slice(11, 23)}Z</time>
            <span className="mr-3 text-muted-foreground">{row.serviceName} · {row.machineName}</span>{row.message}
          </div>;
        })}
      </div>
    </div>
  </div>;
}

function openContainerLogStream(collection: ContainerLogs, query: string, setStatus: (status: string) => void, setErrors: (update: (previous: Record<string, string>) => Record<string, string>) => void) {
  const events = new EventSource(`/api/runtime/logs?${query}`);
  const fail = () => { events.close(); setStatus("Log connection ended. Refresh to reconnect."); };
  events.onopen = () => setStatus("Live");
  events.onerror = fail;
  events.addEventListener("unavailable", fail);
  events.addEventListener("log", (event: MessageEvent<string>) => {
    try {
      const decoded = Schema.decodeUnknownSync(Schema.fromJsonString(containerLogEventSchema))(event.data);
      if (decoded.type === "record") appendContainerLogs(collection, [decoded.record]);
      else setErrors(previous => ({ ...previous, [`${decoded.machineId}/${decoded.containerId}`]: decoded.message }));
    } catch { fail(); }
  });
  return () => events.close();
}

function LogFilter({ label, value, onChange, options }: { label: string; value: string; onChange: (value: string) => void; options: [string, string][] }) {
  return <Select value={value} onValueChange={next => onChange(next ?? "")}>
    <SelectTrigger aria-label={label}><SelectValue>{options.find(([key]) => key === value)?.[1] ?? label}</SelectValue></SelectTrigger>
    <SelectContent><SelectGroup><SelectItem value="">{label}</SelectItem>{options.filter(([key]) => key).map(([key, name]) => <SelectItem key={key} value={key}>{name}</SelectItem>)}</SelectGroup></SelectContent>
  </Select>;
}
