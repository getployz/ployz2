import { CopyButton } from "#/components/copy-button";
import { Skeleton } from "#/components/ui/skeleton";
import { useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import {
  GlobeIcon,
  NetworkIcon,
  PencilIcon,
  PlusIcon,
  Trash2Icon,
  ZapIcon,
} from "lucide-react";
import { Result, Schema, SchemaGetter } from "effect";
import type { ServiceManagedHostname, ServiceRoute } from "#/modules/environment-design/tables";
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
import { Empty, EmptyDescription } from "#/components/ui/empty";
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "#/components/ui/field";
import {
  appFormOptions,
  showErrorsAfterBlurOrSubmit,
  useAppForm,
  validateOnChangeOrBlur,
} from "#/form";
import { cn } from "#/lib/utils";
import { billingKeys } from "#/modules/billing/billing.queries";
import { SERVICE_DEPLOYMENT_DIFF_PATHS } from "#/modules/services/service-deployment-diff/fields";
import {
  serviceManagedHostnameSchema,
  serviceManagedHostnamePrefixSchema,
  servicePrivateDnsSchema,
} from "#/modules/environment-design/services";
import { strictParseOptions } from "#/modules/environment-design/schema";
import { useRuntimeStatus } from "#/providers/runtime-provider";
import { ServiceSettingInput } from "#/routes/_protected/cloud/$organizationSlug/_project/$projectSlug/$environmentSlug/services/$serviceId/-components/ServiceSettingInput";
import type { ServiceDrawerState } from "#/routes/_protected/cloud/$organizationSlug/_project/$projectSlug/$environmentSlug/services/$serviceId/-components/useServiceDrawerState";
import { CustomDomainDialog } from "#/routes/_protected/cloud/$organizationSlug/_project/$projectSlug/$environmentSlug/services/$serviceId/-components/CustomDomainDialog";
import { domainPortSchema } from "./domain-port";


/** Derive a valid managed-domain prefix from the service's private DNS name. */
function defaultPrefix(privateDns: string) {
  const label = privateDns
    .toLowerCase()
    .replace(/[^a-z0-9-]/g, "-")
    .replace(/^-+|-+$/g, "");
  return label.length > 0 ? label : "app";
}

/** First `base`, `base-2`, `base-3`… not already taken. */
function nextFreePrefix(base: string, taken: Iterable<string>) {
  const used = new Set(taken);
  let candidate = base;
  for (let n = 2; used.has(candidate); n += 1) candidate = `${base}-${n}`;
  return candidate;
}

export function ServiceNetworkingSection({
  state,
}: {
  state: ServiceDrawerState;
}) {
  const { service, collection, diff, managedPrefixesInUse, defaultTargetPort } =
    state;
  const routesDiff = diff.field(SERVICE_DEPLOYMENT_DIFF_PATHS.routes);
  const managedDiff = diff.field(SERVICE_DEPLOYMENT_DIFF_PATHS.managedHostnames);
  const privateDnsDiff = diff.field(SERVICE_DEPLOYMENT_DIFF_PATHS.privateDns);
  // `{service}.internal` resolves within the caller's project; the bare name also works via the search domain.
  const privateHostname = `${service.privateDns}.internal`;
  const routes = service.routes;
  const managedList = service.managedHostnames;
  const queryClient = useQueryClient();

  const runtimeStatus = useRuntimeStatus();
  const hostedDnsHostname = runtimeStatus.hostedDnsHostname;
  const hostedDnsHostnameIsCurrent = runtimeStatus.lensStatus === "observed";
  const certificateEvidence = (hostname: string): DomainCertificateEvidence => {
    const certificate = runtimeStatus.certificates.find(
      (candidate) => candidate.hostname === hostname,
    );
    const incomplete = runtimeStatus.incompleteIds.certificates.includes(hostname);
    if (!certificate && !incomplete) return null;
    return {
      status: certificate?.status ?? null,
      lastObserved: !hostedDnsHostnameIsCurrent,
      incomplete,
    };
  };

  const [editing, setEditing] = useState<
    | { kind: "managed"; index: number }
    | { kind: "generate" }
    | { kind: "private" }
    | { kind: "route"; index: number }
    | { kind: "add" }
    | null
  >(null);

  async function commitRoutes(next: ServiceRoute[]): Promise<void> {
    await collection.update(service.id, (draft) => {
      draft.routes = next;
    }).isPersisted.promise;
  }

  function refreshBillingState() {
    return queryClient.invalidateQueries({
      queryKey: billingKeys.state(state.organizationSlug),
      refetchType: "active",
    });
  }

  function commitManaged(next: ServiceManagedHostname[]) {
    void collection.update(service.id, (draft) => {
      draft.managedHostnames = next;
    }).isPersisted.promise;
  }

  // Prefixes are unique per environment, including this service's other domains.
  const takenPrefixesFor = (index: number | null) => [
    ...managedPrefixesInUse,
    ...managedList.filter((_, i) => i !== index).map((m) => m.prefix),
  ];

  const hasAnyDomain = managedList.length > 0 || routes.length > 0;
  const routeBeingEdited =
    editing?.kind === "route" ? routes[editing.index] : undefined;

  return (
    <FieldGroup>
      <Field
        data-changed={managedDiff.changed || routesDiff.changed || undefined}
      >
        <FieldLabel>Public Networking</FieldLabel>
        <FieldDescription>
          Access your application over HTTP with the following domains.
        </FieldDescription>

        <div className="flex flex-col gap-2">
          {!hasAnyDomain ? (
            <Empty>
              <EmptyDescription>No public domains yet.</EmptyDescription>
            </Empty>
          ) : null}

          {managedList.map((managed, index) => (
            <ManagedDomainRow
              key={managed.prefix}
              managed={managed}
              hostedDnsHostname={hostedDnsHostname}
              hostedDnsHostnameIsCurrent={hostedDnsHostnameIsCurrent}
              certificateEvidence={
                hostedDnsHostname
                  ? certificateEvidence(`${managed.prefix}.${hostedDnsHostname}`)
                  : null
              }
              takenPrefixes={takenPrefixesFor(index)}
              defaultTargetPort={defaultTargetPort}
              changed={managedDiff.changed}
              editing={editing?.kind === "managed" && editing.index === index}
              onEdit={() => setEditing({ kind: "managed", index })}
              onCancel={() => setEditing(null)}
              onDelete={() => {
                commitManaged(managedList.filter((_, i) => i !== index));
                setEditing(null);
              }}
              onSubmit={(next) => {
                commitManaged(managedList.map((m, i) => (i === index ? next : m)));
                setEditing(null);
              }}
            />
          ))}

          {editing?.kind === "generate" ? (
            <ManagedDomainDialog
              key="generate-managed-domain"
              mode="generate"
              managed={{
                prefix: nextFreePrefix(defaultPrefix(service.privateDns), takenPrefixesFor(null)),
                targetPort: null,
              }}
              hostedDnsHostname={hostedDnsHostname}
              takenPrefixes={takenPrefixesFor(null)}
              defaultTargetPort={defaultTargetPort}
              onClose={() => setEditing(null)}
              onSubmit={(next) => {
                commitManaged([...managedList, next]);
                setEditing(null);
              }}
            />
          ) : null}

          {routes.map((route, index) => (
            <CustomDomainRow
              key={`${route.hostname}:${route.targetPort}`}
              route={route}
              defaultTargetPort={defaultTargetPort}
              certificateEvidence={certificateEvidence(route.hostname)}
              changed={routesDiff.changed}
              onEdit={() => setEditing({ kind: "route", index })}
              onDelete={() => {
                commitRoutes(routes.filter((_, i) => i !== index));
                setEditing(null);
              }}
            />
          ))}
        </div>

        <div className="flex flex-wrap gap-2">
          {/* Authoring needs no server; the host is filled in once one is enrolled. */}
          <Button
            type="button"
            variant="outline"
            onClick={() => setEditing({ kind: "generate" })}
          >
            <ZapIcon data-icon="inline-start" />
            Generate Domain
          </Button>
          <Button
            type="button"
            variant="outline"
            onClick={() => setEditing({ kind: "add" })}
          >
            <PlusIcon data-icon="inline-start" />
            Custom Domain
          </Button>
        </div>

        {editing?.kind === "add" ? (
          <CustomDomainDialog
            key="add-custom-domain"
            defaultTargetPort={defaultTargetPort}
            capabilityAction={
              <Button
                nativeButton={false}
                render={
                  <Link
                    to="/cloud/$organizationSlug/~/billing"
                    params={{ organizationSlug: state.organizationSlug }}
                  />
                }
                size="sm"
              >
                Review billing
              </Button>
            }
            onCapabilityRejected={refreshBillingState}
            onClose={() => setEditing(null)}
            onSubmit={(next) => commitRoutes([...routes, next])}
          />
        ) : null}
        {editing?.kind === "route" && routeBeingEdited !== undefined ? (
          <CustomDomainDialog
            key={`edit-custom-domain-${editing.index}`}
            route={routeBeingEdited}
            defaultTargetPort={defaultTargetPort}
            capabilityAction={
              <Button
                nativeButton={false}
                render={
                  <Link
                    to="/cloud/$organizationSlug/~/billing"
                    params={{ organizationSlug: state.organizationSlug }}
                  />
                }
                size="sm"
              >
                Review billing
              </Button>
            }
            onCapabilityRejected={refreshBillingState}
            onClose={() => setEditing(null)}
            onSubmit={(next) =>
              commitRoutes(
                routes.map((currentRoute, index) =>
                  index === editing.index ? next : currentRoute,
                ),
              )
            }
          />
        ) : null}
      </Field>

      <Field data-changed={privateDnsDiff.changed || undefined}>
        <FieldLabel>Private Networking</FieldLabel>
        <FieldDescription>
          Communicate with this service from within the environment.
        </FieldDescription>
        <DomainRowShell
          icon={<NetworkIcon />}
          changed={privateDnsDiff.changed}
          actions={
            <>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label="Edit private endpoint"
                onClick={() => setEditing({ kind: "private" })}
              >
                <PencilIcon />
              </Button>
            </>
          }
        >
          <DomainTitle hostname={privateHostname} copyLabel="Copy private hostname" />
          <div className="truncate text-muted-foreground text-sm">
            → or just <span className="font-mono">{service.privateDns}</span>
          </div>
        </DomainRowShell>
        {editing?.kind === "private" ? (
          <Dialog open onOpenChange={(open) => !open && setEditing(null)}>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>Edit private endpoint</DialogTitle>
                <DialogDescription>
                  The name other services in this environment use to reach it.
                </DialogDescription>
              </DialogHeader>
              <ServiceSettingInput
                ariaLabel="Private endpoint name"
                placeholder="api"
                value={service.privateDns}
                isChanged={privateDnsDiff.changed}
                baselineLabel={privateDnsDiff.baselineLabel}
                baselineValue={privateDnsDiff.baselineValue}
                validate={(raw) => {
                  const parsed = Schema.decodeUnknownResult(servicePrivateDnsSchema)(
                    raw,
                    strictParseOptions,
                  );
                  return Result.isFailure(parsed)
                    ? parsed.failure instanceof Error
                      ? parsed.failure.message
                      : "Invalid value"
                    : null;
                }}
                onCommit={(raw) => {
                  const tx = collection.update(service.id, (draft) => {
                    draft.privateDns = raw;
                  });
                  void tx.isPersisted.promise.then(() => setEditing(null));
                  return tx;
                }}
              />
            </DialogContent>
          </Dialog>
        ) : null}
      </Field>
    </FieldGroup>
  );
}

/** Hostname with its copy button right beside it, same spot in every row. */
function DomainTitle({ hostname, copyLabel }: { hostname: string; copyLabel: string }) {
  return (
    <div className="flex min-w-0 items-center gap-1">
      <span className="truncate font-mono text-sm">{hostname}</span>
      <CopyButton value={hostname} label={copyLabel} size="icon-xs" />
    </div>
  );
}

function DomainRowShell({
  icon,
  children,
  changed,
  actions,
}: {
  icon: ReactNode;
  children: ReactNode;
  changed?: boolean;
  actions: ReactNode;
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-3 rounded-lg border bg-card p-3",
        changed && "border-changed-border bg-changed-soft",
      )}
    >
      <span className="text-muted-foreground">{icon}</span>
      <div className="min-w-0 flex-1">{children}</div>
      <div className="flex items-center gap-1">{actions}</div>
    </div>
  );
}

type DomainCertificateEvidence =
  | {
      status: string | null;
      lastObserved: boolean;
      incomplete: boolean;
    }
  | null;

function ManagedDomainRow({
  managed,
  hostedDnsHostname,
  hostedDnsHostnameIsCurrent,
  certificateEvidence,
  takenPrefixes,
  defaultTargetPort,
  changed,
  editing,
  onEdit,
  onCancel,
  onDelete,
  onSubmit,
}: {
  managed: ServiceManagedHostname;
  hostedDnsHostname: string | null;
  hostedDnsHostnameIsCurrent: boolean;
  certificateEvidence: DomainCertificateEvidence;
  takenPrefixes: string[];
  defaultTargetPort: number | null;
  changed: boolean;
  editing: boolean;
  onEdit: () => void;
  onCancel: () => void;
  onDelete: () => void;
  onSubmit: (next: ServiceManagedHostname) => void;
}) {
  const hostname = hostedDnsHostname
    ? `${managed.prefix}.${hostedDnsHostname}`
    : null;
  const port = managed.targetPort ?? defaultTargetPort;

  return (
    <div className="flex flex-col gap-1">
      {editing ? (
        <ManagedDomainDialog
          managed={managed}
          hostedDnsHostname={hostedDnsHostname}
          takenPrefixes={takenPrefixes}
          defaultTargetPort={defaultTargetPort}
          onClose={onCancel}
          onSubmit={onSubmit}
        />
      ) : null}
      <DomainRowShell
        changed={changed}
        icon={<GlobeIcon />}
        actions={
          <>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label="Edit managed domain"
              onClick={onEdit}
            >
              <PencilIcon />
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label="Remove managed domain"
              onClick={onDelete}
            >
              <Trash2Icon />
            </Button>
          </>
        }
      >
        {hostname ? (
          <DomainTitle hostname={hostname} copyLabel="Copy domain" />
        ) : (
          <div className="truncate font-mono text-sm">
            {managed.prefix}
            <span className="text-muted-foreground">
              .<Skeleton variant="inline" aria-label="pending" />.up.ployz.dev
            </span>
          </div>
        )}
        <div className="text-muted-foreground text-sm">→ {port === null ? "Uses PORT" : `Port ${port}`}</div>
      </DomainRowShell>
      <FieldDescription>
        {hostname && !hostedDnsHostnameIsCurrent
          ? "Last seen address; the server is not connected right now."
          : null}
      </FieldDescription>
      <CertificateEvidence evidence={certificateEvidence} />
    </div>
  );
}

function CertificateEvidence({
  evidence,
}: {
  evidence: DomainCertificateEvidence;
}) {
  if (!evidence) return null;

  return (
    <FieldDescription>
      {evidence.status
        ? `${evidence.lastObserved ? "Last observed" : "Observed"} certificate status: ${evidence.status.replaceAll("_", " ")}.`
        : "Certificate status was not observed."}
      {evidence.incomplete
        ? " This observation also lists this certificate as incomplete."
        : null}
    </FieldDescription>
  );
}

function ManagedDomainDialog({
  mode = "edit",
  managed,
  hostedDnsHostname,
  takenPrefixes,
  defaultTargetPort,
  onClose,
  onSubmit,
}: {
  /** "generate" asks only for the port; the subdomain is derived and editable later. */
  mode?: "edit" | "generate";
  managed: ServiceManagedHostname;
  hostedDnsHostname: string | null;
  takenPrefixes: string[];
  defaultTargetPort: number | null;
  onClose: () => void;
  onSubmit: (next: ServiceManagedHostname) => void;
}) {
  const taken = new Set(takenPrefixes);
  const schema = Schema.toStandardSchemaV1(
    Schema.Struct({
      prefix: serviceManagedHostnamePrefixSchema.check(
        Schema.makeFilter<string>((value) =>
          taken.has(value) ? "This subdomain is already in use." : undefined,
        ),
      ),
      port: domainPortSchema,
    })
      .pipe(
        Schema.decodeTo(serviceManagedHostnameSchema, {
          decode: SchemaGetter.transform(({ prefix, port }) => ({
            prefix,
            targetPort: port,
          })),
          encode: SchemaGetter.transform(({ prefix, targetPort }) => ({
            prefix,
            port: targetPort,
          })),
        }),
    ),
    { parseOptions: strictParseOptions },
  );

  const formOptions = appFormOptions.strictSchema({
    defaultValues: {
      prefix: managed.prefix,
      port: managed.targetPort === null ? "" : String(managed.targetPort),
    },
    errorVisibility: showErrorsAfterBlurOrSubmit,
    validators: [validateOnChangeOrBlur(schema)],
  });
  const form = useAppForm({
    ...formOptions,
    onSubmit: ({ schemaOutputs }) => {
      onSubmit(schemaOutputs[0]);
    },
  });

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <form.AppForm>
          <form.Form className="flex flex-col gap-4">
            <DialogHeader>
              <DialogTitle>
                {mode === "generate" ? "Generate Service Domain" : "Edit managed domain"}
              </DialogTitle>
              <DialogDescription>
                {mode === "generate"
                  ? "Enter the port your app is listening on."
                  : "Update your domain or target port."}
              </DialogDescription>
            </DialogHeader>
            <FieldGroup>
              {mode === "edit" ? (
                <form.Field name="prefix">
                  {(field) => (
                    <field.Text
                      label="Subdomain"
                      className="font-mono"
                      description={`.${hostedDnsHostname ?? "{pending}.up.ployz.dev"}`}
                    />
                  )}
                </form.Field>
              ) : null}
              <form.Field name="port">
                {(field) => (
                  <field.Text
                    label={mode === "generate" ? "Port" : "Target port"}
                    type="number"
                    inputMode="numeric"
                    min={1}
                    max={65535}
                    step={1}
                    placeholder={defaultTargetPort === null ? "Uses PORT" : String(defaultTargetPort)}
                    description="Leave blank to use PORT."
                  />
                )}
              </form.Field>
            </FieldGroup>
            <DialogFooter>
              <DialogClose
                render={
                  <Button
                    type="button"
                    variant="outline"
                    // Keep focus on the input so Cancel doesn't trigger blur validation.
                    onMouseDown={(event) => event.preventDefault()}
                  />
                }
              >
                Cancel
              </DialogClose>
              <form.SubmitButton>
                {mode === "generate" ? "Generate Domain" : "Save domain"}
              </form.SubmitButton>
            </DialogFooter>
          </form.Form>
        </form.AppForm>
      </DialogContent>
    </Dialog>
  );
}

function CustomDomainRow({
  route,
  defaultTargetPort,
  certificateEvidence,
  changed,
  onEdit,
  onDelete,
}: {
  route: ServiceRoute;
  defaultTargetPort: number | null;
  certificateEvidence: DomainCertificateEvidence;
  changed: boolean;
  onEdit: () => void;
  onDelete: () => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <DomainRowShell
        changed={changed}
        icon={<GlobeIcon />}
        actions={
          <>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={`Edit ${route.hostname}`}
              onClick={onEdit}
            >
              <PencilIcon />
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={`Remove ${route.hostname}`}
              onClick={onDelete}
            >
              <Trash2Icon />
            </Button>
          </>
        }
      >
        <DomainTitle hostname={route.hostname} copyLabel={`Copy ${route.hostname}`} />
        <div className="text-muted-foreground text-sm">
          → {route.targetPort === null && defaultTargetPort === null
            ? "Uses PORT"
            : `Port ${route.targetPort ?? defaultTargetPort}`}
        </div>
      </DomainRowShell>
      <CertificateEvidence evidence={certificateEvidence} />
    </div>
  );
}
