import { useReducedMotion } from "#/lib/motion";
import { createContext, useContext, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { ArrowLeftIcon, Maximize2Icon, Minimize2Icon, XIcon } from "lucide-react";
import { Button } from "#/components/ui/button";
import { buttonVariants } from "#/components/ui/button-variants";
import { DashboardNavigationPicker } from "#/components/dashboard-navigation";
import { ENVIRONMENT_INDEX_ROUTE_TO } from "./environment-route-paths";

// Shared by the frame and header; route selection remains owned by the router.
export const InspectorPresentation = createContext<{
  takeover: boolean;
  canResize: boolean;
  isMobile: boolean;
  toggleFullscreen: () => void;
} | null>(null);

type CanvasInspectorHeaderParams = {
  organizationSlug: string;
  projectSlug: string;
  environmentSlug: string;
};

export function CanvasInspectorHeader({ params, children }: {
  params: CanvasInspectorHeaderParams;
  children: ReactNode;
}) {
  const presentation = useContext(InspectorPresentation);
  const reducedMotion = useReducedMotion();
  if (!presentation) throw new Error("Canvas inspector header must be inside its workspace");
  const { takeover, canResize, isMobile, toggleFullscreen } = presentation;
  const returnLink = (
    <Link
      to={ENVIRONMENT_INDEX_ROUTE_TO}
      params={params}
      search={(previous) => ({ ...previous, tab: undefined })}
      viewTransition={reducedMotion ? false : { types: ["canvas-inspector-close"] }}
      className={buttonVariants({ variant: "ghost", size: "icon" })}
      data-canvas-inspector-exit
      data-canvas-inspector-desktop-control={isMobile ? undefined : true}
      aria-label={isMobile || takeover ? "Back to Architecture" : "Close inspector"}
      title={isMobile || takeover ? "Back to Architecture" : "Close inspector"}
    >
      {isMobile || takeover ? <ArrowLeftIcon /> : <XIcon />}
    </Link>
  );

  return (
    <div className="canvas-inspector-header flex shrink-0 items-center gap-3 border-b px-4">
      {isMobile || takeover ? returnLink : null}
      <div className="min-w-0 flex-1">{children}</div>
      {isMobile ? <>
        <span aria-hidden className="text-muted-foreground">/</span>
        <DashboardNavigationPicker scope={{ kind: "environment", ...params }} />
      </> : null}
      {!isMobile && canResize ? (
        <Button
          variant="ghost"
          size="icon"
          data-canvas-inspector-desktop-control
          onClick={toggleFullscreen}
          aria-label={takeover ? "Restore inspector" : "Fill canvas"}
          title={takeover ? "Restore inspector" : "Fill canvas"}
        >
          {takeover ? <Minimize2Icon /> : <Maximize2Icon />}
        </Button>
      ) : null}
      {!isMobile && !takeover ? returnLink : null}
    </div>
  );
}
