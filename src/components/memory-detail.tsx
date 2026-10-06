import { useState } from "react";
import { EyeIcon, Trash2Icon } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { ScrollArea } from "@/components/ui/scroll-area";
import { formatDate, formatTime } from "@/lib/format";
import type { MemoryRow } from "@/lib/types";

interface MemoryDetailProps {
  memory: MemoryRow | null;
  onDelete?: (id: number) => void;
}

export function MemoryDetail({
  memory,
  onDelete,
}: MemoryDetailProps) {
  const [confirmDeleteId, setConfirmDeleteId] = useState<number | null>(null);
  if (memory === null) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <EyeIcon />
          </EmptyMedia>
          <EmptyTitle>Select a memory</EmptyTitle>
          <EmptyDescription>
            Pick anything from the list to read it, or search your timeline.
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <article className="flex min-h-0 min-w-0 flex-1 flex-col gap-6">
      <div className="flex items-start gap-2">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <h2 className="font-heading text-xl font-semibold tracking-[-0.02em]">
            {memory.title}
          </h2>
          <p className="mt-1 text-xs text-muted-foreground">
            {memory.app ?? memory.source} · {formatDate(memory.createdAt)} at{" "}
            {formatTime(memory.createdAt)}
          </p>
        </div>
        {onDelete ? (confirmDeleteId === memory.id ? <div className="flex items-center gap-1"><Button variant="ghost" size="sm" onClick={() => setConfirmDeleteId(null)}>Cancel</Button><Button variant="destructive" size="sm" onClick={() => { onDelete(memory.id); setConfirmDeleteId(null); }}>Delete memory</Button></div> :
          <Button variant="ghost" size="icon-sm" aria-label="Delete memory" onClick={() => setConfirmDeleteId(memory.id)}><Trash2Icon /></Button>
        ) : null}
      </div>
      <ScrollArea className="min-h-0 flex-1 border-t pt-5">
        {memory.enrichment ? (
          <div className="max-w-[70ch] space-y-6 pb-8 text-sm">
            <div>
              <p className="leading-relaxed">{memory.enrichment.structured.summary}</p>
            </div>
            {([
              ["Decisions", memory.enrichment.structured.decisions],
              ["Commitments", memory.enrichment.structured.commitments],
              ["Action items", memory.enrichment.structured.actionItems?.map((item) => item.content) ?? []],
            ] as const).filter(([, values]) => values.length > 0).map(([label, values]) => (
              <div key={label}>
                <p className="mb-1 text-xs font-medium text-muted-foreground">{label}</p>
                <ul className="list-disc space-y-1 pl-5">
                  {values.map((value) => <li key={value}>{value}</li>)}
                </ul>
              </div>
            ))}
            <details className="border-t pt-4">
              <summary className="cursor-pointer text-xs font-medium text-muted-foreground">Show captured source</summary>
              <pre className="mt-3 max-w-full whitespace-pre-wrap break-words text-sm leading-relaxed text-foreground/80">{memory.content}</pre>
              <p className="mt-3 text-xs text-muted-foreground">Processed with {memory.enrichment.provider} · {memory.enrichment.model}</p>
            </details>
          </div>
        ) : (
          <pre className="max-w-full whitespace-pre-wrap break-words text-sm leading-relaxed text-foreground/90">
            {memory.content}
          </pre>
        )}
      </ScrollArea>
    </article>
  );
}
