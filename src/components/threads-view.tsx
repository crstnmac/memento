import { useEffect, useState } from "react";
import { FolderTreeIcon, HistoryIcon, Loader2Icon } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { api } from "@/lib/api";
import { formatDate, formatTime, timeAgo } from "@/lib/format";
import { useMemento } from "@/lib/store";
import type { MemoryRow, MemoryVersion, ThreadRow } from "@/lib/types";

interface EraItem {
  id: number;
  title?: string;
  createdAt?: number;
}

export function ThreadsView() {
  const { versionsSignal } = useMemento();
  const [threads, setThreads] = useState<ThreadRow[]>([]);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [items, setItems] = useState<MemoryRow[]>([]);
  const [eras, setEras] = useState<MemoryVersion[]>([]);
  const [loading, setLoading] = useState(false);

  async function load() {
    setLoading(true);
    try {
      setThreads(await api.listThreads(100));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    load();
  }, []);

  useEffect(() => {
    if (expanded !== null) load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [versionsSignal]);

  async function toggle(id: number) {
    if (expanded === id) {
      setExpanded(null);
      setItems([]);
      setEras([]);
      return;
    }
    setExpanded(id);
    setItems([]);
    setEras([]);
    const thread = await api.getThread(id);
    if (thread) setItems(thread[1]);
    setEras(await api.getThreadVersions(id));
  }

  function eraPreview(version: MemoryVersion): string {
    try {
      const parsed = JSON.parse(version.content) as EraItem[];
      const first = parsed[0];
      return first?.title ?? `${parsed.length} items`;
    } catch {
      return "Snapshot";
    }
  }

  return (
    <div className="flex min-w-0 flex-1 flex-col px-4 pb-4">
      <div className="flex items-center gap-3 pt-4 pb-3">
        <h1 className="text-lg font-semibold tracking-tight">Threads</h1>
        <Badge variant="outline">{threads.length}</Badge>
        <span className="min-w-0 flex-1" />
        <Button variant="ghost" size="sm" onClick={load} disabled={loading}>
          Refresh
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {threads.length === 0 ? (
          <Empty>
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <FolderTreeIcon />
              </EmptyMedia>
              <EmptyTitle>No threads yet</EmptyTitle>
              <EmptyDescription>
                Sustained stints on one app are grouped into threads as you
                work, each with a frozen-per-hour memory history.
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <div className="flex flex-col gap-1.5">
            {threads.map((thread) => {
              const open = expanded === thread.id;
              return (
                <div key={thread.id} className="rounded-lg border bg-card">
                  <button
                    type="button"
                    onClick={() => toggle(thread.id)}
                    aria-expanded={open}
                    className="flex w-full cursor-pointer items-start gap-2.5 rounded-lg p-2.5 text-left outline-none hover:bg-muted/50 focus-visible:ring-[3px] focus-visible:ring-ring/50"
                  >
                    <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <div className="flex items-center gap-1.5">
                        <span className="text-sm font-medium">
                          {thread.title}
                        </span>
                        {thread.openCount > 0 ? (
                          <Badge className="bg-warning/10 text-warning">
                            {thread.openCount} open
                          </Badge>
                        ) : null}
                        <span className="min-w-0 flex-1" />
                        <span className="shrink-0 text-xs text-muted-foreground">
                          {timeAgo(thread.lastActivity)}
                        </span>
                      </div>
                      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                        <span>{thread.app ?? thread.source}</span>
                        <span>·</span>
                        <span>{thread.itemCount} moments</span>
                        {thread.derivative ? (
                          <>
                            <span>·</span>
                            <span className="truncate">{thread.derivative}</span>
                          </>
                        ) : null}
                      </div>
                    </div>
                    {open ? (
                      <Loader2Icon className="size-4 animate-spin text-muted-foreground" />
                    ) : null}
                  </button>
                  {open ? (
                    <div className="flex flex-col gap-2 border-t p-2">
                      {eras.length > 0 ? (
                        <div className="flex flex-col gap-1.5">
                          <div className="flex items-center gap-1.5 px-1 text-xs font-medium text-muted-foreground">
                            <HistoryIcon className="size-3.5" />
                            Eras of state — frozen per hour
                          </div>
                          {eras.map((era) => (
                            <div
                              key={era.id}
                              className="rounded-md border bg-muted/20 px-2.5 py-1.5"
                            >
                              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                                <span className="font-medium text-foreground/80">
                                  {formatDate(era.createdAt)} ·{" "}
                                  {formatTime(era.createdAt)}
                                </span>
                                {era.isFrozen ? (
                                  <Badge variant="secondary">Frozen</Badge>
                                ) : (
                                  <Badge className="bg-success/10 text-success">
                                    Live
                                  </Badge>
                                )}
                                <Badge variant="outline">
                                  {era.wordCount} words
                                </Badge>
                                <Badge variant="outline">
                                  {era.itemCount} items
                                </Badge>
                              </div>
                              <p className="truncate text-xs text-foreground/70">
                                {eraPreview(era)}
                              </p>
                            </div>
                          ))}
                        </div>
                      ) : null}
                      {items.length === 0 ? (
                        <p className="text-xs text-muted-foreground">
                          Loading items…
                        </p>
                      ) : (
                        items.map((memory) => (
                          <div
                            key={memory.id}
                            className="rounded-md bg-muted/40 px-2.5 py-1.5"
                          >
                            <div className="flex items-center gap-2 text-xs text-muted-foreground">
                              <span className="truncate">{memory.title}</span>
                              <span className="min-w-0 flex-1" />
                              <span>
                                {formatDate(memory.createdAt)} ·{" "}
                                {formatTime(memory.createdAt)}
                              </span>
                            </div>
                            <p className="truncate text-xs text-foreground/80">
                              {memory.enrichment?.structured.summary ?? memory.content}
                            </p>
                          </div>
                        ))
                      )}
                    </div>
                  ) : null}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
