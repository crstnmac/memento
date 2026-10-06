import { Loader2Icon } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export interface ModelState {
  status: "idle" | "busy" | "ready" | "error";
  progress: number | null;
  error: string | null;
}

export const IDLE_MODEL_STATE: ModelState = {
  status: "idle",
  progress: null,
  error: null,
};

export const READY_MODEL_STATE: ModelState = {
  status: "ready",
  progress: null,
  error: null,
};

export function ModelRow({
  title,
  detail,
  state,
  onDownload,
  onRemove,
  removeArmed,
  onToggleRemove,
}: {
  title: string;
  detail: string;
  state: ModelState;
  onDownload: () => void;
  onRemove?: () => void;
  removeArmed?: boolean;
  onToggleRemove?: () => void;
}) {
  const removable = Boolean(onRemove) && state.status === "ready";
  return (
    <div className="rounded-md border bg-muted/30 px-2.5 py-2">
      <div className="flex items-center gap-2">
        <div className="min-w-0 flex-1">
          <p className="truncate text-xs font-medium">
            {title}{" "}
            <span className="font-normal text-muted-foreground">· {detail}</span>
          </p>
          {state.status === "error" ? (
            <p role="alert" className="mt-0.5 text-xs text-destructive">
              {state.error}
            </p>
          ) : null}
        </div>
        {state.status === "ready" ? (
          <Badge className="bg-success/10 text-success">Ready</Badge>
        ) : state.status === "busy" ? (
          <Badge variant="outline" className="gap-1">
            <Loader2Icon className="size-3 animate-spin" aria-hidden />
            {state.progress !== null ? `${state.progress}%` : "…"}
          </Badge>
        ) : (
          <Button size="sm" variant="outline" onClick={onDownload}>
            Download
          </Button>
        )}
        {removable ? (
          removeArmed ? (
            <Button size="sm" variant="destructive" onClick={onRemove}>
              Confirm remove
            </Button>
          ) : (
            <Button
              size="sm"
              variant="ghost"
              className="text-muted-foreground hover:text-destructive"
              onClick={onToggleRemove}
            >
              Remove
            </Button>
          )
        ) : null}
      </div>
      {state.status === "busy" && state.progress !== null ? (
        <div
          className="mt-1.5 h-1 overflow-hidden rounded-full bg-muted"
          role="progressbar"
          aria-label={`Downloading ${title}`}
          aria-valuenow={state.progress}
          aria-valuemin={0}
          aria-valuemax={100}
        >
          <div
            className={cn("h-full rounded-full bg-primary transition-all")}
            style={{ width: `${state.progress}%` }}
          />
        </div>
      ) : null}
    </div>
  );
}
