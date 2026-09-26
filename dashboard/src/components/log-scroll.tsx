import { Button } from "./ui/button";
import { Skeleton } from "./ui/skeleton";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "./ui/empty";
import { useTimeZone, zoneLabel } from "#/utils/time-zone";
import { cn } from "#/lib/utils";
import { useLayoutEffect, useRef, type ReactNode } from "react";
import { useVirtualizer, type VirtualizerOptions } from "@tanstack/react-virtual";

export function useLogScroll(options: Pick<VirtualizerOptions<HTMLDivElement, HTMLDivElement>, "count" | "getItemKey" | "onChange" | "paddingStart">) {
  const element = useRef<HTMLDivElement>(null);
  const virtual = useVirtualizer({
    ...options,
    getScrollElement: () => element.current,
    estimateSize: () => 24,
    overscan: 12,
    anchorTo: "end",
    followOnAppend: true,
    scrollEndThreshold: 48,
  });
  useLayoutEffect(() => { virtual.scrollToEnd(); }, [virtual]);
  return { element, virtual };
}

/** Log columns' widths, shared by the header, the rows, and their skeleton. */
export const LOG_TIME_COLUMN = { build: "w-24", container: "w-48" } as const;

/** The column header a log reads under; the time column names its zone. */
export function LogHeader({ time, children, latest }: { time: string; children: ReactNode; latest?: ReactNode }) {
  const zone = zoneLabel(useTimeZone());
  return <div className="flex h-8 shrink-0 items-center gap-3 border-b px-1 font-mono text-xs text-muted-foreground">
    <span className={cn("shrink-0", time)}>Time ({zone})</span>
    <span className="min-w-0 flex-1">{children}</span>
    {latest}
  </div>;
}

/** Jumps back to the newest line once the reader has scrolled away from it. */
export function LatestButton({ onClick }: { onClick: () => void }) {
  return <Button variant="ghost" size="xs" onClick={onClick}>Latest</Button>;
}

/** A log with nothing to show: said once, in the middle of where the lines would be. */
export function LogEmpty({ title, children }: { title: string; children?: ReactNode }) {
  return <Empty>
    <EmptyHeader>
      <EmptyTitle>{title}</EmptyTitle>
      {children ? <EmptyDescription>{children}</EmptyDescription> : null}
    </EmptyHeader>
  </Empty>;
}

export function BuildLogViewer({ children }: { children: ReactNode }) {
  // Keep arbitrary output chunks in one measured block so split lines stay intact.
  const { element, virtual } = useLogScroll({ count: 1, getItemKey: () => "build-output" });
  return <>
    <LogHeader time={LOG_TIME_COLUMN.build} latest={virtual.isAtEnd() ? null : <LatestButton onClick={() => virtual.scrollToEnd()} />}>Step</LogHeader>
    <div ref={element} className="min-h-0 flex-1 overflow-auto break-words font-mono text-xs leading-6" tabIndex={0} aria-label="Build logs">
      <div ref={virtual.measureElement} data-index={0}>{children}</div>
    </div>
  </>;
}

const LINE_WIDTHS = ["w-2/5", "w-3/5", "w-1/3", "w-1/2"];

/** Log lines' shape (a time, then the message), held until the lines arrive. */
export function LogSkeleton({ rows = LINE_WIDTHS.length, label, time }: { rows?: number; label: string; time: string }) {
  return <ol aria-busy aria-label={label}>
    {LINE_WIDTHS.slice(0, rows).map((width) => <li key={width} className="flex h-6 items-center gap-3 px-1">
      <Skeleton className={cn("h-3 shrink-0", time)} />
      <Skeleton className={cn("h-3", width)} />
    </li>)}
  </ol>;
}
