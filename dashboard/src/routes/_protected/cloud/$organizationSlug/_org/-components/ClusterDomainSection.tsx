"use client";

import { useTransition } from "react";
import { GlobeIcon } from "lucide-react";
import { toast } from "sonner";
import { CopyButton } from "#/components/copy-button";
import { Button } from "#/components/ui/button";
import { Item, ItemActions, ItemContent, ItemDescription, ItemMedia, ItemTitle } from "#/components/ui/item";
import { Spinner } from "#/components/ui/spinner";

export function ClusterDomainSection({ name, onPublish }: {
  name: string | null;
  onPublish: () => Promise<void>;
}) {
  const [publishing, startPublish] = useTransition();

  function handlePublish() {
    startPublish(async () => {
      try {
        await onPublish();
      } catch (error) {
        toast.error(error instanceof Error ? error.message : "The generated domain couldn’t be published.");
      }
    });
  }

  return (
    <section aria-labelledby="cluster-domain-heading">
      <h2 id="cluster-domain-heading" className="sr-only">Generated domain</h2>
      <Item variant="outline">
        <ItemMedia variant="icon">
          <GlobeIcon />
        </ItemMedia>
        <ItemContent>
          <ItemTitle>Generated domain</ItemTitle>
          <ItemDescription className={name === null ? undefined : "font-mono"}>{name ?? "Not reserved yet"}</ItemDescription>
        </ItemContent>
        <ItemActions>
          {name === null ? null : <CopyButton value={name} label="Copy generated domain" />}
          <Button type="button" variant="outline" disabled={publishing} onClick={handlePublish}>
            {publishing ? <Spinner data-icon="inline-start" /> : null}
            Publish now
          </Button>
        </ItemActions>
      </Item>
    </section>
  );
}
