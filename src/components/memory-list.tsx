import { FolderOpenIcon } from "lucide-react";

import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { timeAgo } from "@/lib/format";
import type { MemoryRow } from "@/lib/types";

interface MemoryListProps {
  memories: MemoryRow[];
  selectedId: number;
  empty: boolean;
  onSelect: (id: number) => void;
  /** When set, a "Show more" button is shown after the last row. */
  onLoadMore?: () => void;
}

export function MemoryList({
  memories,
  selectedId,
  empty,
  onSelect,
  onLoadMore,
}: MemoryListProps) {
  if (empty) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <FolderOpenIcon />
          </EmptyMedia>
          <EmptyTitle>Nothing here yet</EmptyTitle>
          <EmptyDescription>
            Try a different word or phrase.
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <ScrollArea className="h-full min-h-0 flex-1" aria-label="Scrollable memories">
      <div className="flex flex-col">
        {memories.map((memory) => {
          const selected = memory.id === selectedId;
          return (
            <button
              key={memory.id}
              type="button"
              onClick={() => onSelect(memory.id)}
              aria-pressed={selected}
              className={cn("group flex w-full cursor-pointer items-start gap-4 border-b px-1 py-5 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/40", selected ? "text-foreground" : "hover:bg-muted/50")}
            >
              <span
                className={cn(
                  "mt-1.5 size-1.5 shrink-0 rounded-full transition-colors",
                  selected ? "bg-primary" : "bg-border",
                )}
              />
              <span className="flex min-w-0 flex-1 flex-col gap-1">
                <span className="flex w-full items-center gap-2">
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {timeAgo(memory.createdAt)}
                  </span>
                  <span className="truncate text-xs text-muted-foreground">{memory.app ?? memory.source}</span>
                </span>
                <span className="text-sm font-semibold leading-5">{memory.title}</span>
                <span className="line-clamp-2 max-w-2xl text-sm leading-6 text-muted-foreground">
                  {memory.enrichment?.structured.summary ?? memory.content}
                </span>
              </span>
            </button>
          );
        })}
        {onLoadMore ? (
          <div className="flex justify-center py-4">
            <Button variant="outline" size="sm" onClick={onLoadMore}>Show more</Button>
          </div>
        ) : null}
      </div>
    </ScrollArea>
  );
}
