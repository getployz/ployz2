import { createContext, useEffect, useRef, useState, type ReactElement, type ReactNode } from "react";
import { Link, useLoaderData, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import {
  ChevronDownIcon, ChevronRightIcon, CircleCheckIcon, CircleDashedIcon, CircleDotIcon, CircleSlashIcon, CircleXIcon,
} from "lucide-react";
import { CancelDeploymentDialog } from "#/components/cancel-deployment-dialog";
import { useRetryDeployment } from "#/components/deployment-row";
import { Button } from "#/components/ui/button";
import { Drawer, DrawerContent, DrawerTitle, DrawerTrigger } from "#/components/ui/drawer";
import { Popover, PopoverContent, PopoverTitle, PopoverTrigger } from "#/components/ui/popover";
import { useIsMobile } from "#/hooks/use-mobile";
import { cn } from "#/lib/utils";
import type { EnvironmentDeploymentSummary } from "#/modules/deployments/deployment-contract";
import { useEnvironmentDeployments, type DeploymentAttempt } from "#/modules/deployments/deployment.collection";
import { deploymentStatusLabel, type DeploymentView } from "#/modules/deployments/deployment-view";
import { formatRelativeTime } from "#/utils/relative-time";
import { CANVAS_ROUTE_ID, useDeploymentMode } from "./deployment-mode";
import { ENVIRONMENT_ROUTE_FROM } from "./environment-route-paths";
import { useCanvasInspectorSelection } from "./useCanvasInspectorSelection";

const isActive = ({ deployment }: DeploymentAttempt) =>
  deployment.status === "queued" || deployment.status === "planning" || deployment.status === "deploying";
const shortId = (id: string) => id.slice(0, 8);
const segment = "inline-flex h-7 min-w-0 items-center gap-1.5 rounded-md px-2.5 text-xs font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 data-[active=true]:bg-background data-[active=true]:text-foreground data-[active=true]:shadow-xs [&_svg]:size-3.5 [&_svg]:shrink-0";

function StatusIcon({ view }: { view: DeploymentView }) {
  switch (view.status) {
    case "deployed": return <CircleCheckIcon className="text-success" />;
    case "failed": return <CircleXIcon className="text-destructive" />;
    case "cancelled": return <CircleSlashIcon className="text-muted-foreground" />;
    case "queued": return <CircleDashedIcon className="text-muted-foreground" />;
    default: return <CircleDotIcon className="text-info" />;
  }
}

/** The live canvas owns the change state, so it portals the apply zone into this slot of the bar. */
export const ApplyZoneSlot = createContext<HTMLElement | null>(null);

/**
 * The floating deploy bar: Live | Deployments ⌄ on every screen size, usable while a service panel is open.
 * `children` renders after the segments (the apply zone, #1051).
 */
export function DeployBar({ children }: { children?: ReactNode }) {
  const { organizationSlug, environmentSlug } = useParams({ from: ENVIRONMENT_ROUTE_FROM });
  const { environmentId } = useLoaderData({ from: ENVIRONMENT_ROUTE_FROM });
  const listOpen = useSearch({ from: CANVAS_ROUTE_ID, select: (search) => search.deploymentList === true });
  const viewed = useDeploymentMode();
  const attempts = useEnvironmentDeployments(organizationSlug, environmentId);
  const { selectedNodeId } = useCanvasInspectorSelection();
  const navigate = useNavigate();
  const isMobile = useIsMobile();
  const barRef = useRef<HTMLDivElement>(null);
  // The oldest queued or running attempt holds, or is next for, the Environment execution slot; the rest wait behind it.
  const active = attempts.filter(isActive);
  const running = active.at(-1);
  const queued = active.length > 1 ? active[0] : undefined;
  const latest = attempts[0];

  function setListOpen(open: boolean) {
    void navigate({ to: ".", search: (previous) => ({ ...previous, deploymentList: open || undefined }), replace: true });
  }

  // Esc closes the topmost thing: the list, menus and dialogs close themselves (they portal outside the scene), the panel closes itself, then Esc leaves Deployment Mode.
  useEffect(() => {
    if (!viewed || listOpen || selectedNodeId) return;
    function leaveOnEscape(event: KeyboardEvent) {
      if (event.key !== "Escape" || event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
      const scene = barRef.current?.closest(".environment-canvas-scene");
      if (event.target !== document.body && !(event.target instanceof Node && scene?.contains(event.target))) return;
      void navigate({ to: ".", search: (previous) => ({ ...previous, deployment: undefined }) });
    }
    document.addEventListener("keydown", leaveOnEscape);
    return () => document.removeEventListener("keydown", leaveOnEscape);
  }, [viewed, listOpen, selectedNodeId, navigate]);

  const listTrigger = (
    <button type="button" className={segment} data-active={viewed !== null} aria-label={viewed ? `Deployment ${shortId(viewed.deployment.id)}, all deployments` : "Deployments"}>
      {viewed ? <><StatusIcon view={viewed.view} /><span className="font-mono">{shortId(viewed.deployment.id)}</span></>
        : <>Deployments{latest ? <span className="font-normal text-muted-foreground max-[860px]:hidden">
          · {latest.view.status} {formatRelativeTime(latest.deployment.finishedAt ?? latest.deployment.createdAt, undefined, "narrow")}
        </span> : null}</>}
      <ChevronDownIcon />
    </button>
  );
  // A queued or running attempt opens directly from Live Mode, with no list.
  const openRunning = !viewed && running ? (
    <Link to="." search={(previous) => ({ ...previous, deployment: running.deployment.id, deploymentList: undefined })}
      className={cn(segment, "bg-info-soft text-info hover:text-info")}>
      <StatusIcon view={running.view} />
      <span className="tabular-nums">{running.view.status === "deploying" ? `Deploying ${running.view.deployed}/${running.view.changed}` : deploymentStatusLabel(running.view)}</span>
      <ChevronRightIcon />
    </Link>
  ) : null;
  const openQueued = !viewed && queued ? (
    <Link to="." search={(previous) => ({ ...previous, deployment: queued.deployment.id, deploymentList: undefined })} className={segment}>
      <CircleDashedIcon />{active.length > 2 ? `${active.length - 1} queued` : "Queued"}
    </Link>
  ) : null;
  const list = <DeploymentList attempts={attempts} viewedId={viewed?.deployment.id ?? null} environmentSlug={environmentSlug} />;

  return (
    <div ref={barRef} role="group" aria-label="Deploy bar" className="deploy-bar" data-deployment={viewed ? "" : undefined}>
      <div className="flex min-w-0 items-center gap-0.5 rounded-lg bg-muted p-0.5">
        <Link to="." search={(previous) => ({ ...previous, deployment: undefined, deploymentList: undefined })}
          className={segment} data-active={viewed === null}>
          Live
        </Link>
        {openRunning}
        {openQueued}
        {isMobile ? (
          <Drawer open={listOpen} onOpenChange={setListOpen}>
            {openRunning ? null : <DrawerTrigger render={listTrigger} />}
            <DrawerContent><DrawerTitle className="sr-only">Deployments</DrawerTitle>{list}</DrawerContent>
          </Drawer>
        ) : (
          <Popover open={listOpen} onOpenChange={setListOpen}>
            {openRunning ? null : <PopoverTrigger render={listTrigger} />}
            <PopoverContent anchor={barRef} side="top" sideOffset={8} padding="none" className="w-[min(26rem,calc(100vw-2rem))]">
              <PopoverTitle className="sr-only">Deployments</PopoverTitle>
              {list}
            </PopoverContent>
          </Popover>
        )}
      </div>
      {viewed ? <DeploymentActions deployment={viewed.deployment} /> : null}
      {children}
    </div>
  );
}

/** Retry on a failed attempt, Cancel on a queued or running one; both keep their existing semantics. */
function DeploymentActions({ deployment }: { deployment: EnvironmentDeploymentSummary }) {
  const { organizationSlug } = useParams({ from: ENVIRONMENT_ROUTE_FROM });
  const { retryDeployment, isRetrying } = useRetryDeployment(deployment);
  const [cancelOpen, setCancelOpen] = useState(false);
  const cancellable = !deployment.cancellationRequestedAt && ["queued", "planning", "deploying"].includes(deployment.status);
  return <>
    {deployment.canRetry ? <Button size="sm" variant="outline" disabled={isRetrying} onClick={() => void retryDeployment()}>Retry</Button> : null}
    {cancellable ? <Button size="sm" variant="outline" onClick={() => setCancelOpen(true)}>Cancel</Button> : null}
    <CancelDeploymentDialog open={cancelOpen} onOpenChange={setCancelOpen} organizationSlug={organizationSlug} deployment={deployment} />
  </>;
}

/** Live first, then the environment's deployments newest first. */
function DeploymentList({ attempts, viewedId, environmentSlug }: { attempts: DeploymentAttempt[]; viewedId: string | null; environmentSlug: string }) {
  return (
    <nav aria-label="Deployments" className="max-h-[min(28rem,70dvh)] overflow-y-auto p-1.5">
      <ListRow current={viewedId === null} search={{ deployment: undefined }}
        icon={<span className="mx-0.75 mt-1.5 size-2 shrink-0 rounded-full bg-success" />} title="Live" detail={`${environmentSlug} as it is now`} />
      <p className="px-2.5 pt-2 pb-1 text-xs font-medium text-muted-foreground">Deployments</p>
      {attempts.length === 0 ? <p className="px-2.5 py-2 text-sm text-muted-foreground">No deployments yet</p> : null}
      {attempts.map(({ deployment, view }) => (
        <ListRow key={deployment.id} current={deployment.id === viewedId} search={{ deployment: deployment.id }}
          icon={<StatusIcon view={view} />}
          title={<><span className="font-mono">{shortId(deployment.id)}</span> · {deployment.message ?? "Deployment"}</>}
          detail={`${deploymentStatusLabel(view)} · ${formatRelativeTime(deployment.createdAt)}`} />
      ))}
    </nav>
  );
}

function ListRow({ current, search, icon, title, detail }: {
  current: boolean; search: { deployment: string | undefined }; icon: ReactElement; title: ReactNode; detail: string;
}) {
  return (
    <Link to="." search={(previous) => ({ ...previous, ...search, deploymentList: undefined })} data-current={current}
      className="flex items-start gap-2.5 rounded-md px-2.5 py-2 text-sm outline-none hover:bg-muted focus-visible:bg-muted data-[current=true]:bg-muted [&_svg]:mt-0.5 [&_svg]:size-3.5 [&_svg]:shrink-0">
      {icon}
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium">{title}</span>
        <span className="block truncate text-xs text-muted-foreground">{detail}</span>
      </span>
    </Link>
  );
}
