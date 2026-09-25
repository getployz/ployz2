import { Background, BackgroundVariant, ReactFlow, ReactFlowProvider } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { eq, useLiveSuspenseQuery } from "@tanstack/react-db";
import { useParams } from "@tanstack/react-router";
import { useCollectionScope } from "#/collections/use-collection-scope";
import { getCanvasPositionsCollection, getRawServicesCollection } from "#/collections/collections";
import type { DeploymentAttempt } from "#/modules/deployments/deployment.collection";
import { parseVolumeConfig } from "#/modules/environment-design/volume-config";
import { ENVIRONMENT_ROUTE_FROM } from "../environment-route-paths";
import { BackToLive } from "../deployment-mode";
import { canvasNodeTypes } from "./canvas-node-types";
import { SERVICE_NODE_HEIGHT, SERVICE_NODE_WIDTH } from "./constants";
import { DeploymentNodeCard, DeploymentNodeLink } from "./DeploymentNode";
import type { CanvasDeploymentNode } from "./types";

/**
 * The canvas as one Cloud Deployment Attempt saw it: exactly its node set at current positions.
 * Read-only by construction: no dragging, context menus, Create, or change controls. Service nodes open the read-only panel.
 */
export function DeploymentCanvas({ attempt, environmentId }: { attempt: DeploymentAttempt; environmentId: string }) {
  const { organizationSlug } = useParams({ from: ENVIRONMENT_ROUTE_FROM });
  const scope = useCollectionScope();
  const positions = getCanvasPositionsCollection(organizationSlug, scope);
  const services = getRawServicesCollection(organizationSlug, scope);
  const { data: positionRows } = useLiveSuspenseQuery({
    queryKey: ["deployment-canvas-positions", positions.id, environmentId],
    query: (q) => q.from({ position: positions }).where(({ position }) => eq(position.environmentId, environmentId))
      .select(({ position }) => ({ resourceType: position.resourceType, resourceId: position.resourceId, x: position.x, y: position.y })),
  });
  const { data: serviceRows } = useLiveSuspenseQuery({
    queryKey: ["deployment-canvas-services", services.id, environmentId],
    query: (q) => q.from({ service: services }).where(({ service }) => eq(service.environmentId, environmentId))
      .select(({ service }) => ({ id: service.id, name: service.name })),
  });
  const nodes = attempt.nodes.flatMap((node): CanvasDeploymentNode[] => {
    const view = attempt.view.nodes.find((candidate) => candidate.nodeId === node.nodeId);
    if (!view) return [];
    const position = positionRows.find((row) => row.resourceType === node.nodeType && row.resourceId === node.nodeId);
    const name = node.nodeType === "volume" ? parseVolumeConfig(node.config).name
      : serviceRows.find((row) => row.id === node.nodeId)?.name ?? node.nodeId;
    return [{
      id: node.nodeId, type: "deployment", position: { x: position?.x ?? 0, y: position?.y ?? 0 },
      width: SERVICE_NODE_WIDTH, height: SERVICE_NODE_HEIGHT, draggable: false,
      data: { nodeId: node.nodeId, nodeType: node.nodeType, name, config: node.config, view },
    }];
  });

  return (
    <div className="canvas-graph" data-deployment-canvas>
      <div className="hidden h-full min-[861px]:block">
        <ReactFlowProvider initialNodes={nodes} initialWidth={1200} initialHeight={800} fitView initialMaxZoom={1.25}>
          <ReactFlow
            nodes={nodes}
            edges={[]}
            nodeTypes={canvasNodeTypes}
            nodesDraggable={false}
            nodesConnectable={false}
            elementsSelectable={false}
            nodesFocusable={false}
            fitView
            proOptions={{ hideAttribution: true }}
            minZoom={0.4}
            maxZoom={1.35}
          >
            <Background variant={BackgroundVariant.Dots} gap={16} size={1} />
          </ReactFlow>
        </ReactFlowProvider>
      </div>
      <div className="canvas-node-list absolute inset-0 overflow-y-auto px-4 pb-4 pt-16 min-[861px]:hidden">
        <div className="flex flex-col gap-3">
          {nodes.map((node) => <DeploymentNodeLink key={node.id} data={node.data}><DeploymentNodeCard data={node.data} /></DeploymentNodeLink>)}
        </div>
      </div>
      <BackToLive className="absolute top-4 right-4 z-10 min-[861px]:hidden" />
    </div>
  );
}
