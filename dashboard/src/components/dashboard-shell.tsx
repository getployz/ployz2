import type { ReactNode } from "react";
import { useMatch } from "@tanstack/react-router";
import { useCollectionScope } from "#/collections/use-collection-scope";
import { AppSidebar } from "./app-sidebar";
import type { DashboardScope } from "./dashboard-navigation-model";
import { DashboardPageHeader } from "./dashboard-header";
import { MobileDashboardNavigation } from "./dashboard-navigation";
import { NavigationProgress } from "./navigation-progress";
import { OrganizationCollectionRefreshNotice } from "./organization-collection-refresh-notice";
import { SidebarProvider, useSidebar } from "./ui/sidebar";
import { cn } from "#/lib/utils";

export function DashboardShell({
  scope,
  children,
}: {
  scope: DashboardScope;
  children: ReactNode;
}) {
  return (
    <SidebarProvider className="h-dvh min-h-0 overflow-hidden">
      <DashboardLayout scope={scope}>{children}</DashboardLayout>
    </SidebarProvider>
  );
}

function DashboardLayout({
  scope,
  children,
}: {
  scope: DashboardScope;
  children: ReactNode;
}) {
  const { open, isMobile } = useSidebar();
  const collectionScope = useCollectionScope();
  const canvas = useMatch({
    from: "/_protected/cloud/$organizationSlug/_project/$projectSlug/$environmentSlug/_canvas",
    shouldThrow: false,
  });
  return (
    <>
      {!isMobile ? (
        <aside
          className={cn(
            "hidden shrink-0 border-r bg-sidebar motion-safe:transition-[width] motion-safe:duration-150 min-wf-nav:block",
            open ? "w-64" : "w-16",
          )}
          aria-label="Dashboard navigation"
        >
          <AppSidebar scope={scope} />
        </aside>
      ) : null}
      <main className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
        {isMobile ? (
          <MobileDashboardNavigation
            key={
              scope.kind === "all"
                ? scope.organizationSlug
                : `${scope.organizationSlug}/${scope.projectSlug}/${scope.environmentSlug}`
            }
            scope={scope}
          />
        ) : null}
        {!canvas ? <DashboardPageHeader /> : null}
        <NavigationProgress />
        <OrganizationCollectionRefreshNotice
          scope={collectionScope}
          organizationSlug={scope.organizationSlug}
        />
        <div
          data-scroll-restoration-id="wireframe-content"
          className="min-h-0 min-w-0 flex-1 overflow-y-auto"
        >
          {children}
        </div>
      </main>
    </>
  );
}
