import { eq, useLiveQuery } from "@tanstack/react-db";
import { withoutVirtualProps } from "#/lib/tanstack-db";
import { useQuery } from "@tanstack/react-query";
import { linkOptions } from "@tanstack/react-router";
import { variableGroupsEnabled } from "#/lib/feature-flags";
import { useCollectionScope } from "#/collections/use-collection-scope";
import { preloadCollection } from "#/collections/query-collection";
import {
  getProjectsCollection,
  getEnvironmentsCollection,
  getRawServicesCollection,
  getRawEnvironmentResourcesCollection,
  getResourceLineagesCollection,
  getCanvasPositionsCollection,
  getEnvironmentNodeConfigSnapshotsCollection,
  getVolumeRemoveAttemptsCollection,
} from "#/collections/collections";
import {
  getServicesCollection,
  getEnvironmentResourcesCollection,
  getVolumeResourcesCollection,
  type EnvironmentParams,
} from "#/modules/services/services.collection";
import type { ServicePage } from "../services/$serviceId/-components/service-pages";
import {
  ENVIRONMENT_SERVICE_ROUTE_TO,
  ENVIRONMENT_RESOURCE_ROUTE_TO,
} from "./environment-route-paths";

export type NavigationNode = {
  id: string;
  name: string;
  type: "service" | "volume" | "variable_group";
};

export function nodeDestination(
  params: EnvironmentParams,
  node: NavigationNode,
  page?: ServicePage,
) {
  const { organizationSlug, projectSlug, environmentSlug } = params;
  return node.type === "service"
    ? linkOptions({
        to: ENVIRONMENT_SERVICE_ROUTE_TO,
        params: {
          organizationSlug,
          projectSlug,
          environmentSlug,
          serviceId: node.id,
        },
        search: { tab: page },
      })
    : linkOptions({
        to: ENVIRONMENT_RESOURCE_ROUTE_TO,
        params: {
          organizationSlug,
          projectSlug,
          environmentSlug,
          resourceId: node.id,
        },
        search: {},
      });
}

export function useEnvironmentNavigationNodes(
  params: EnvironmentParams,
  enabled: boolean,
) {
  const scope = useCollectionScope();
  const { organizationSlug, projectSlug, environmentSlug } = params;
  const ready = useQuery({
    queryKey: [
      "node-navigation",
      scope.sessionId,
      scope.userId,
      organizationSlug,
    ],
    enabled,
    staleTime: Infinity,
    queryFn: async () => {
      // Reuse lifecycle projections: a Volume remains navigable until removal completes.
      await Promise.all([
        preloadCollection(getProjectsCollection(organizationSlug, scope)),
        preloadCollection(getEnvironmentsCollection(organizationSlug, scope)),
        preloadCollection(getRawServicesCollection(organizationSlug, scope)),
        preloadCollection(
          getRawEnvironmentResourcesCollection(organizationSlug, scope),
        ),
        preloadCollection(
          getResourceLineagesCollection(organizationSlug, scope),
        ),
        preloadCollection(
          getCanvasPositionsCollection(organizationSlug, scope),
        ),
        preloadCollection(
          getEnvironmentNodeConfigSnapshotsCollection(organizationSlug, scope),
        ),
        preloadCollection(
          getVolumeRemoveAttemptsCollection(organizationSlug, scope),
        ),
      ]);
      return true;
    },
  });
  const services = ready.data
    ? getServicesCollection(organizationSlug, scope)
    : null;
  const resources = ready.data
    ? getEnvironmentResourcesCollection(organizationSlug, scope)
    : null;
  const volumes = ready.data
    ? getVolumeResourcesCollection(organizationSlug, scope)
    : null;
  const serviceRows = useLiveQuery(
    (q) =>
      services
        ? q
            .from({ service: services })
            .where(({ service }) => eq(service.projectSlug, projectSlug))
            .where(({ service }) =>
              eq(service.environmentSlug, environmentSlug),
            )
            .select(({ service }) => ({ id: service.id, name: service.name }))
        : undefined,
    [services, projectSlug, environmentSlug],
  );
  const resourceRows = useLiveQuery(
    (q) =>
      resources
        ? q
            .from({ resource: resources })
            .where(({ resource }) => eq(resource.projectSlug, projectSlug))
            .where(({ resource }) =>
              eq(resource.environmentSlug, environmentSlug),
            )
            .select(({ resource }) => ({
              id: resource.resource.id,
              name: resource.resource.name,
            }))
        : undefined,
    [resources, projectSlug, environmentSlug],
  );
  const volumeRows = useLiveQuery(
    (q) =>
      volumes
        ? q
            .from({ volume: volumes })
            .where(({ volume }) => eq(volume.projectSlug, projectSlug))
            .where(({ volume }) => eq(volume.environmentSlug, environmentSlug))
            .select(({ volume }) => ({
              id: volume.resource.id,
              name: volume.resource.name,
            }))
        : undefined,
    [volumes, projectSlug, environmentSlug],
  );

  const nodes: NavigationNode[] = [
    ...(serviceRows.data ?? []).map(withoutVirtualProps).map(({ id, name }) => ({
      id,
      name,
      type: "service" as const,
    })),
    ...(variableGroupsEnabled ? (resourceRows.data ?? []) : []).map(withoutVirtualProps).map(
      ({ id, name }) => ({ id, name, type: "variable_group" as const }),
    ),
    ...(volumeRows.data ?? []).map(withoutVirtualProps).map(({ id, name }) => ({
      id,
      name,
      type: "volume" as const,
    })),
  ];
  return {
    nodes,
    isLoading:
      ready.isPending ||
      serviceRows.isLoading ||
      resourceRows.isLoading ||
      volumeRows.isLoading,
    isError:
      ready.isError ||
      serviceRows.isError ||
      resourceRows.isError ||
      volumeRows.isError,
    retry: () => void ready.refetch(),
  };
}
