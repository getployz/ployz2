import { useEffect, useRef } from "react";
import { ArrowLeftIcon } from "lucide-react";
import { Button } from "#/components/ui/button";
import { InputGroup, InputGroupInput } from "#/components/ui/input-group";
import type {
  CanvasEnvironmentChangeGroup,
} from "#/modules/environment-design/canvas-environment-change-state";
import { ApplyChangeGroupCard } from "#/routes/_protected/cloud/$organizationSlug/_project/$projectSlug/$environmentSlug/-components/canvas/ApplyChangeGroupCard";

export function EnvironmentChangesReview({
  groups,
  totalChanges,
  canDeploy,
  commitMessage,
  canSave,
  onSave,
  onDiscardAll,
  onClose,
  onCommitMessageChange,
  onDeploy,
  onDiscardNode,
  onDiscardRow,
}: {
  groups: CanvasEnvironmentChangeGroup[];
  totalChanges: number;
  canDeploy: boolean;
  commitMessage: string;
  canSave: boolean;
  onSave: () => void;
  onDiscardAll: () => void;
  onClose: () => void;
  onCommitMessageChange: (value: string) => void;
  onDeploy: () => void;
  onDiscardNode: (group: CanvasEnvironmentChangeGroup) => void;
  onDiscardRow: (group: CanvasEnvironmentChangeGroup, path: string) => void;
}) {
  const backRef = useRef<HTMLButtonElement>(null);
  useEffect(() => { backRef.current?.focus({ preventScroll: true }); }, []);

  return (
    <section
      className="canvas-change-review"
      aria-label="Environment changes"
      onKeyDown={(event) => {
        if (event.key === "Escape" && !event.defaultPrevented) {
          event.preventDefault();
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="flex shrink-0 items-center gap-3 border-b px-6 py-4">
        <Button ref={backRef} variant="ghost" size="icon" aria-label="Back to editing" title="Back to editing" onClick={onClose}>
          <ArrowLeftIcon />
        </Button>
        <div>
          <h2 className="font-medium">Environment changes</h2>
          <p className="text-sm text-muted-foreground">
            {canSave ? "Unpublished configuration" : totalChanges > 0 ? "Configuration saved · not yet deployed" : "No changes to review"}
          </p>
        </div>
      </div>
      <div className="shrink-0 border-b px-6 py-3">
        <InputGroup>
          <InputGroupInput
            aria-label="Commit message"
            placeholder="Commit message (optional)"
            value={commitMessage}
            onChange={(event) => onCommitMessageChange(event.target.value)}
          />
        </InputGroup>
      </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-6">
          <div className="flex flex-col gap-3">
            {groups.map((group) => (
              <ApplyChangeGroupCard
                key={`${group.nodeType}:${group.nodeId}`}
                group={group}
                totalChanges={totalChanges}
                visibleGroupCount={groups.length}
                onCloseDialog={onClose}
                onDiscardNode={() => onDiscardNode(group)}
                onDiscardRow={(_, path) => onDiscardRow(group, path)}
              />
            ))}
          </div>
        </div>

      <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 border-t px-6 py-4">
        {groups.some(group => group.canDiscard) ? <Button variant="ghost" className="mr-auto" onClick={onDiscardAll}>Discard all changes</Button> : null}
        <Button variant="outline" disabled={!canSave} onClick={onSave}>Save without deploying</Button>
        {canDeploy ? <Button onClick={onDeploy}>Deploy changes</Button> : null}
      </div>
    </section>
  );
}
