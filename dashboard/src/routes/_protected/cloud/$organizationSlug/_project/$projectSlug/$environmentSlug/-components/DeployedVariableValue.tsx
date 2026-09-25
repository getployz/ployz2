import { useState } from "react";
import { EyeIcon, EyeOffIcon } from "lucide-react";
import { Button } from "#/components/ui/button";
import { Skeleton } from "#/components/ui/skeleton";

const MASK = "••••••••";

/** Hidden until revealed; null is Sealed and never revealable; undefined is still loading. */
export function DeployedVariableValue({ name, value, from }: { name: string; value: string | null | undefined; from: string[] }) {
  const [revealed, setRevealed] = useState(false);
  return (
    <div className="flex min-w-0 flex-col">
      <div className="flex min-w-0 items-center gap-1.5">
        {value === undefined ? <Skeleton variant="inline" /> : value === null ? <span className="text-muted-foreground">Sealed</span> : (
          <>
            <span className="min-w-0 break-all text-muted-foreground">{revealed ? value : MASK}</span>
            <Button type="button" variant="ghost" size="icon-xs" aria-label={`${revealed ? "Hide" : "Show"} ${name}`} onClick={() => setRevealed(!revealed)}>
              {revealed ? <EyeOffIcon /> : <EyeIcon />}
            </Button>
          </>
        )}
      </div>
      {from.length > 0 ? <span className="font-sans text-muted-foreground">resolves from {from.join(", ")}</span> : null}
    </div>
  );
}
