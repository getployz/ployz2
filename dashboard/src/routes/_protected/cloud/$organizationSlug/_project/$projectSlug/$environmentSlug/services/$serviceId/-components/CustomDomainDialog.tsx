import {
  decodeStrict,
  strictParseOptions,
} from "#/modules/environment-design/schema";
import { useState, type ReactNode } from "react";
import { CircleAlertIcon } from "lucide-react";
import { Schema, SchemaGetter } from "effect";
import type { ServiceRoute } from "#/modules/environment-design/tables";
import { asRecord } from "#/lib/json";
import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from "#/components/ui/alert";
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
import { FieldGroup } from "#/components/ui/field";
import {
  appFormOptions,
  showErrorsAfterBlurOrSubmit,
  useAppForm,
  validateOnChangeOrBlur,
} from "#/form";
import { serviceRouteSchema } from "#/modules/environment-design/services";
import { domainPortSchema } from "./domain-port";

const customDomainFormSchema = Schema.toStandardSchemaV1(
  Schema.Struct({
    hostname: Schema.Trim.check(
      Schema.isNonEmpty({ message: "Enter a hostname." }),
    ),
    port: domainPortSchema,
  }).pipe(
    Schema.decodeTo(
      Schema.Struct({
        hostname: Schema.String,
        targetPort: Schema.NullOr(Schema.Int.check(
          Schema.isBetween({ minimum: 1, maximum: 65_535 }),
        )),
      }),
      {
        decode: SchemaGetter.transform(({ hostname, port }) => ({
          hostname,
          targetPort: port,
        })),
        encode: SchemaGetter.transform(({ hostname, targetPort }) => ({
          hostname,
          port: targetPort,
        })),
      },
    ),
  ),
  { parseOptions: strictParseOptions },
);

const customDomainFormOptions = appFormOptions.strictSchema({
  defaultValues: { hostname: "", port: "" },
  errorVisibility: showErrorsAfterBlurOrSubmit,
  validators: [validateOnChangeOrBlur(customDomainFormSchema)],
});

type SaveFailure = {
  kind: "capability" | "persistence";
  message: string;
};

function isCustomDomainCapabilityError<T>(
  error: T,
): error is T & { _tag: "CustomDomainCapabilityError"; message?: string } {
  return asRecord(error)?.["_tag"] === "CustomDomainCapabilityError";
}

function errorMessage<T>(error: T) {
  return error instanceof Error
    ? error.message
    : "The custom domain could not be saved.";
}

export function CustomDomainDialog({
  route,
  defaultTargetPort,
  capabilityAction,
  onCapabilityRejected,
  onClose,
  onSubmit,
}: {
  route?: ServiceRoute;
  defaultTargetPort: number | null;
  capabilityAction: ReactNode;
  onCapabilityRejected: () => Promise<void>;
  onClose: () => void;
  onSubmit: (next: ServiceRoute) => Promise<void>;
}) {
  const [saveFailure, setSaveFailure] = useState<SaveFailure | null>(null);

  const form = useAppForm({
    ...customDomainFormOptions,
    defaultValues: {
      hostname: route?.hostname ?? "",
      port: route?.targetPort == null ? "" : String(route.targetPort),
    },
    onSubmit: async ({ schemaOutputs }) => {
      setSaveFailure(null);
      try {
        await onSubmit(
          decodeStrict(serviceRouteSchema, {
            id: route?.id ?? crypto.randomUUID(),
            ...schemaOutputs[0],
          }),
        );
        onClose();
      } catch (error) {
        if (isCustomDomainCapabilityError(error)) {
          setSaveFailure({
            kind: "capability",
            message:
              error.message ??
              "Your custom-domain access changed. Review billing before saving.",
          });
          try {
            await onCapabilityRejected();
          } catch {
            // The authoritative save failure remains visible if billing refresh fails.
          }
          return;
        }
        setSaveFailure({ kind: "persistence", message: errorMessage(error) });
      }
    },
  });

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <form.AppForm>
          <form.Form className="flex flex-col gap-4">
            <DialogHeader>
              <DialogTitle>
                {route ? "Edit custom domain" : "Add custom domain"}
              </DialogTitle>
              <DialogDescription>
                Point a domain you own at this service.
              </DialogDescription>
            </DialogHeader>
            <FieldGroup>
              <form.Field name="hostname">
                {(field) => (
                  <field.Text
                    id="custom-domain-hostname"
                    label="Domain"
                    className="font-mono"
                    placeholder="api.example.com"
                  />
                )}
              </form.Field>
              <form.Field name="port">
                {(field) => (
                  <field.Text
                    id="custom-domain-target-port"
                    label="Target port"
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
            {saveFailure ? (
              <Alert variant="destructive">
                <CircleAlertIcon />
                <AlertTitle>
                  {saveFailure.kind === "capability"
                    ? "Custom domain access changed"
                    : "Custom domain not saved"}
                </AlertTitle>
                <AlertDescription>{saveFailure.message}</AlertDescription>
                {saveFailure.kind === "capability" ? (
                  <AlertAction>{capabilityAction}</AlertAction>
                ) : null}
              </Alert>
            ) : null}
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
              <form.SubmitButton>Save route</form.SubmitButton>
            </DialogFooter>
          </form.Form>
        </form.AppForm>
      </DialogContent>
    </Dialog>
  );
}
