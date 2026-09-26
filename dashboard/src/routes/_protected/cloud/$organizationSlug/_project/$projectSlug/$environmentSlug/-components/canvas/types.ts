import type { Node } from "@xyflow/react";
import type { ServiceConfig } from "@ployz/sdk/config";
import type { AttemptTargetNode, DeploymentNodeView } from "#/modules/deployments/deployment-view";

export type CanvasResourceType = "service" | "volume";

export type CanvasResourceNodeData = {
  resourceType: CanvasResourceType;
  resourceId: string;
  environmentId: string;
};

export type CanvasServiceNodeData = {
  resourceType: "service";
  resourceId: string;
  serviceId: string;
  environmentId: string;
};

export type CanvasServiceNode = Node<CanvasServiceNodeData, "service">;

export type CanvasVolumeNodeData = {
  resourceType: "volume";
  resourceId: string;
  environmentId: string;
};

export type CanvasVolumeNode = Node<CanvasVolumeNodeData, "volume">;

/** A node as a Cloud Deployment Attempt saw it; drawn only in Deployment Mode. */
export type CanvasDeploymentNodeData = {
  /** The node as the attempt's target list froze it. */
  node: AttemptTargetNode;
  name: string;
  /** The service config the attempt deployed; null for volumes and removed services. */
  config: ServiceConfig | null;
  view: DeploymentNodeView;
};

export type CanvasDeploymentNode = Node<CanvasDeploymentNodeData, "deployment">;

export type CanvasResourceNode =
  | CanvasServiceNode
  | CanvasVolumeNode;

export type FlowPosition = { x: number; y: number };
export type CreatorPanel = "root" | "git" | "image";
