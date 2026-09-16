import {
  Background,
  BackgroundVariant,
  ReactFlow,
  ReactFlowProvider,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { createFileRoute } from "@tanstack/react-router";
import { ServiceCreateCommand } from "#/components/service-create-command";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "#/components/ui/card";

export const Route = createFileRoute(
  "/_protected/cloud/$organizationSlug/_project/new",
)({
  component: RouteComponent,
});

function RouteComponent() {
  const { organizationSlug } = Route.useParams();

  return (
    <main className="relative flex min-h-svh items-center justify-center overflow-hidden p-4">
      <div className="pointer-events-none absolute inset-0" aria-hidden="true">
        <ReactFlowProvider>
          <ReactFlow
            nodes={[]}
            edges={[]}
            nodesDraggable={false}
            nodesConnectable={false}
            panOnDrag={false}
            zoomOnScroll={false}
            proOptions={{ hideAttribution: true }}
          >
            <Background variant={BackgroundVariant.Dots} gap={16} size={1} />
          </ReactFlow>
        </ReactFlowProvider>
      </div>
      <Card className="relative w-full max-w-md">
        <CardHeader>
          <CardTitle>Add your app</CardTitle>
          <CardDescription>
            Pick the GitHub repository. Ployz reads it and fills in the rest.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <ServiceCreateCommand
            organizationSlug={organizationSlug}
            initialPanel="git"
          />
        </CardContent>
      </Card>
    </main>
  );
}
