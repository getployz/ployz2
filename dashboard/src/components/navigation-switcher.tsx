import { type ReactNode, useState, useSyncExternalStore } from "react";
import { useLiveQuery } from "@tanstack/react-db";
import { useMutation, useQuery } from "@tanstack/react-query";
import {
  Link,
  useNavigate,
  useParams,
  useRouter,
} from "@tanstack/react-router";
import { useServerFn } from "@tanstack/react-start";
import {
  Building2Icon,
  CheckIcon,
  ChevronDownIcon,
  FolderIcon,
  PlusIcon,
} from "lucide-react";
import { getEnvironmentsCollection } from "#/collections/collections";
import { useCollectionScope } from "#/collections/use-collection-scope";
import {
  getDashboardDestination,
  type DashboardSection,
} from "#/components/dashboard-navigation-model";
import { useDashboardSection } from "#/components/use-dashboard-section";
import { Button } from "#/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "#/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "#/components/ui/dropdown-menu";
import {
  Field,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "#/components/ui/field";
import { Input } from "#/components/ui/input";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/utils";
import { createEnvironmentServerFn } from "#/modules/environment-design/workspace-functions";
import {
  organizationStateQueryOptions,
  projectListQueryOptions,
} from "#/modules/environment-design/workspace-queries";

type Projection = "desktop" | "mobile" | "rail";

function ScopePicker({
  label,
  suffix,
  name,
  icon,
  projection = "desktop",
  triggerVariant = "ghost",
  children,
}: {
  label: string;
  suffix?: string;
  name: string;
  icon: ReactNode;
  projection?: Projection;
  triggerVariant?: "ghost" | "outline";
  children: ReactNode;
}) {
  const rail = projection === "rail";
  const fullLabel = suffix ? `${label} / ${suffix}` : label;
  const trigger = (
    <DropdownMenuTrigger
      openOnHover={rail}
      render={
        <Button
          variant={triggerVariant}
          size={rail ? "icon-sm" : "sm"}
          aria-label={`${name}: ${fullLabel}`}
          title={fullLabel}
          className={cn(!rail && "w-full min-w-0 justify-start")}
        />
      }
    >
      {projection !== "mobile" ? icon : null}
      {!rail && (
        <>
          <span className="flex min-w-0 flex-1 items-center gap-1 text-left">
            <span className="min-w-0 flex-1 truncate">{label}</span>
            {suffix ? <>
              <span aria-hidden className="shrink-0 text-muted-foreground">/</span>
              <span className="max-w-[65%] shrink-0 truncate">{suffix}</span>
            </> : null}
          </span>
          <ChevronDownIcon data-icon="inline-end" />
        </>
      )}
    </DropdownMenuTrigger>
  );
  return (
    <DropdownMenu>
      {trigger}
      <DropdownMenuContent
        align="start"
        side={rail ? "right" : "bottom"}
        className="min-w-56 max-w-[calc(100vw-2rem)]"
      >
        {children}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function ProjectSwitcher({
  organizationSlug,
  projectSlug,
  environmentSlug,
  section,
  triggerLabel,
  triggerVariant,
  projection = "desktop",
}: {
  organizationSlug: string;
  projectSlug?: string;
  environmentSlug?: string;
  section: DashboardSection;
  triggerLabel?: string;
  triggerVariant?: "ghost" | "outline";
  projection?: Projection;
}) {
  const collectionScope = useCollectionScope();
  const {
    data: projects = [],
    isPending,
    isError,
    refetch,
  } = useQuery(projectListQueryOptions(organizationSlug));
  const environmentCollection = getEnvironmentsCollection(
    organizationSlug,
    collectionScope,
  );
  const { data: environments = [], isLoading: environmentsLoading } =
    useLiveQuery(environmentCollection);
  const environmentsError = useSyncExternalStore(
    (onChange) =>
      collectionScope.queryClient.getQueryCache().subscribe(onChange),
    () => environmentCollection.utils.isError,
    () => false,
  );
  const [createProject, setCreateProject] = useState<string | null>(null);
  const activeProject = projects.find(
    (project) => project.slug === projectSlug,
  );
  const activeEnvironment = environments.find(
    (environment) =>
      environment.projectId === activeProject?.id &&
      environment.namespace === environmentSlug,
  );
  const label = projectSlug ? activeProject?.name ?? projectSlug : "Organization";
  const environmentLabel = projectSlug
    ? activeEnvironment?.name ?? environmentSlug ?? activeProject?.resolvedEnvironment?.name ?? "Choose environment"
    : undefined;

  return (
    <>
      <ScopePicker
        name="Project and environment"
        label={triggerLabel ?? label}
        suffix={triggerLabel ? undefined : environmentLabel}
        icon={<FolderIcon data-icon="inline-start" />}
        projection={projection}
        triggerVariant={triggerVariant}
      >
        <DropdownMenuGroup>
          <DropdownMenuItem
            render={
              <Link
                {...getDashboardDestination(
                  { kind: "all", organizationSlug },
                  section,
                )}
              />
            }
          >
            <Building2Icon />
            Organization
            {!projectSlug && <CheckIcon className="ml-auto" />}
          </DropdownMenuItem>
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        {isPending || isError ? (
          <DropdownMenuGroup>
            <DropdownMenuItem
              disabled={isPending}
              onClick={() => void refetch()}
            >
              {isPending
                ? "Loading projects…"
                : "Could not load projects. Retry"}
            </DropdownMenuItem>
          </DropdownMenuGroup>
        ) : projects.length === 0 ? (
          <DropdownMenuGroup>
            <DropdownMenuItem disabled>No projects yet</DropdownMenuItem>
          </DropdownMenuGroup>
        ) : (
          projects.map((project) => {
            const projectEnvironments = environments.filter(
              (environment) => environment.projectId === project.id,
            );
            return (
              <DropdownMenuGroup key={project.id}>
                <DropdownMenuLabel>{project.name}</DropdownMenuLabel>
                {projectEnvironments.map((environment) => (
                  <DropdownMenuItem
                    key={environment.id}
                    render={
                      <Link
                        {...getDashboardDestination(
                          {
                            kind: "environment",
                            organizationSlug,
                            projectSlug: project.slug,
                            environmentSlug: environment.namespace,
                          },
                          section,
                        )}
                      />
                    }
                  >
                    {environment.name}
                    {project.slug === projectSlug &&
                      environment.namespace === environmentSlug && (
                        <CheckIcon className="ml-auto" />
                      )}
                  </DropdownMenuItem>
                ))}
                {projectEnvironments.length === 0 && (
                  <DropdownMenuItem
                    disabled={!environmentsError}
                    onClick={() => void environmentCollection.utils.refetch()}
                  >
                    {environmentsError
                      ? "Could not load environments. Retry"
                      : environmentsLoading
                        ? "Loading environments…"
                        : "No environments yet"}
                  </DropdownMenuItem>
                )}
                <DropdownMenuItem
                  onClick={() => setCreateProject(project.slug)}
                >
                  <PlusIcon /> Add environment
                </DropdownMenuItem>
              </DropdownMenuGroup>
            );
          })
        )}
        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuItem
            render={
              <Link
                to="/cloud/$organizationSlug/new"
                params={{ organizationSlug }}
                search={{}}
              />
            }
          >
            <PlusIcon /> New project
          </DropdownMenuItem>
        </DropdownMenuGroup>
      </ScopePicker>
      {createProject && (
        <CreateEnvironmentDialog
          key={`${organizationSlug}/${createProject}`}
          onOpenChange={(open) => {
            if (!open) setCreateProject(null);
          }}
          organizationSlug={organizationSlug}
          projectSlug={createProject}
          section={section}
        />
      )}
    </>
  );
}

function CreateEnvironmentDialog({
  onOpenChange,
  organizationSlug,
  projectSlug,
  section,
}: {
  onOpenChange: (open: boolean) => void;
  organizationSlug: string;
  projectSlug: string;
  section: DashboardSection;
}) {
  const collectionScope = useCollectionScope();
  const [name, setName] = useState("");
  const router = useRouter();
  const navigate = useNavigate();
  const createEnvironment = useServerFn(createEnvironmentServerFn);
  const mutation = useMutation({
    mutationFn: (input: {
      organizationSlug: string;
      projectSlug: string;
      name: string;
      locationKey: string | undefined;
    }) =>
      createEnvironment({
        data: {
          organizationSlug: input.organizationSlug,
          projectSlug: input.projectSlug,
          name: input.name,
        },
      }),
    onSuccess: async (receipt, input) => {
      await getEnvironmentsCollection(
        input.organizationSlug,
        collectionScope,
      ).writeCommitted(receipt.data);
      // A completed creation still belongs to its original scope after navigation.
      if (router.state.location.state.key !== input.locationKey) return;
      onOpenChange(false);
      await navigate(
        getDashboardDestination(
          {
            kind: "environment",
            organizationSlug: input.organizationSlug,
            projectSlug: input.projectSlug,
            environmentSlug: receipt.data.namespace,
          },
          section,
        ),
      );
    },
  });

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add environment</DialogTitle>
          <DialogDescription>
            Create an empty environment with no services or variables.
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (name.trim() && !mutation.isPending)
              mutation.mutate({
                organizationSlug,
                projectSlug,
                name: name.trim(),
                locationKey: router.state.location.state.key,
              });
          }}
        >
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="env-name">Name</FieldLabel>
              <Input
                id="env-name"
                placeholder="staging"
                value={name}
                onChange={(event) => setName(event.target.value)}
                autoFocus
              />
            </Field>
            {mutation.isError && (
              <FieldError>{mutation.error.message}</FieldError>
            )}
          </FieldGroup>
          <DialogFooter className="mt-4">
            <DialogClose render={<Button variant="outline" />}>
              Cancel
            </DialogClose>
            <Button type="submit" disabled={!name.trim() || mutation.isPending}>
              {mutation.isPending && <Spinner data-icon="inline-start" />}
              Add environment
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function NavigationSwitcher({
  projection = "desktop",
}: {
  projection?: Projection;
}) {
  const { organizationSlug, projectSlug, environmentSlug } = useParams({
    strict: false,
  });
  const section = useDashboardSection();
  const {
    data: organizationState,
    isPending,
    isError,
    refetch,
  } = useQuery(organizationStateQueryOptions(organizationSlug));
  if (!organizationSlug) return null;
  const organizationLabel =
    organizationState?.activeOrganization?.name ?? organizationSlug;

  return (
    <div
      className={cn(
        "flex min-w-0 gap-1",
        projection === "mobile"
          ? "w-full [&>*]:min-w-0 [&>*]:flex-1 [&>*:first-child]:max-w-[30%]"
          : "flex-col",
        projection === "rail" && "items-center",
      )}
    >
      <ScopePicker
        name="Organization"
        label={organizationLabel}
        icon={<Building2Icon data-icon="inline-start" />}
        projection={projection}
      >
        <DropdownMenuGroup>
          <DropdownMenuLabel>Organizations</DropdownMenuLabel>
          {organizationState?.organizations.map((organization) => (
            <DropdownMenuItem
              key={organization.id}
              render={
                <Link
                  {...getDashboardDestination(
                    { kind: "all", organizationSlug: organization.slug },
                    section,
                  )}
                />
              }
            >
              {organization.name}
              {organization.slug === organizationSlug && (
                <CheckIcon className="ml-auto" />
              )}
            </DropdownMenuItem>
          ))}
          {isPending || isError ? (
            <DropdownMenuItem
              disabled={isPending}
              onClick={() => void refetch()}
            >
              {isPending
                ? "Loading organizations…"
                : "Could not load organizations. Retry"}
            </DropdownMenuItem>
          ) : null}
        </DropdownMenuGroup>
      </ScopePicker>
      <ProjectSwitcher
        key={`${organizationSlug}/${projectSlug ?? ""}/${environmentSlug ?? ""}`}
        organizationSlug={organizationSlug}
        projectSlug={projectSlug && environmentSlug ? projectSlug : undefined}
        environmentSlug={environmentSlug}
        section={section}
        projection={projection}
      />
    </div>
  );
}
