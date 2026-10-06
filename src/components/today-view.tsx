import { useMemo } from "react";
import {
  CheckIcon,
  CircleIcon,
  FileTextIcon,
  GlobeIcon,
  MessageCircleIcon,
  StickyNoteIcon,
  VideoIcon,
} from "lucide-react";

import { DaySummary } from "@/components/day-summary";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { formatTime } from "@/lib/format";
import { useMemento } from "@/lib/store";
import type { ActionItem, MemoryRow } from "@/lib/types";

const previewMemories: MemoryRow[] = [
  { id: -1, source: "capture", app: "Google Chrome", title: "Reviewed the product roadmap", content: "Focused on milestones, dependencies, and the next release.", createdAt: new Date().setHours(9, 14, 0, 0), isMeeting: false, openLoop: false },
  { id: -2, source: "capture", app: "WhatsApp", title: "Caught up with Priya", content: "Moved the afternoon sync and compared notes on the client update.", createdAt: new Date().setHours(10, 2, 0, 0), isMeeting: false, openLoop: true },
  { id: -3, source: "meeting", app: "Google Meet", title: "Team sync", content: "Aligned on onboarding improvements, launch sequencing, and next steps.", createdAt: new Date().setHours(10, 30, 0, 0), isMeeting: true, openLoop: true },
  { id: -4, source: "note", app: "Memento", title: "A note to myself", content: "Follow up with Alex about the design feedback.", createdAt: new Date().setHours(11, 42, 0, 0), isMeeting: false, openLoop: true },
];

const previewActions: ActionItem[] = [
  { id: -1, memoryId: -1, content: "Share the roadmap update with leadership", status: "open", createdAt: Date.now(), completedAt: null, isUrgent: false, isUnread: false, sortOrder: 0, remindAt: new Date().setHours(14, 0, 0, 0), remindDate: null, remindSlot: null, reminderShown: false },
  { id: -2, memoryId: -3, content: "Follow up with Alex on design feedback", status: "open", createdAt: Date.now(), completedAt: null, isUrgent: false, isUnread: false, sortOrder: 1, remindAt: null, remindDate: null, remindSlot: null, reminderShown: false },
];

function startOfToday() {
  const now = new Date();
  return new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
}

function sourceIcon(memory: MemoryRow) {
  const source = `${memory.source} ${memory.app ?? ""}`.toLowerCase();
  if (memory.isMeeting || source.includes("meeting")) return VideoIcon;
  if (source.includes("whatsapp") || source.includes("slack") || source.includes("message")) return MessageCircleIcon;
  if (source.includes("chrome") || source.includes("safari") || source.includes("browser")) return GlobeIcon;
  if (source.includes("note")) return StickyNoteIcon;
  return FileTextIcon;
}

function summary(memory: MemoryRow) {
  return memory.enrichment?.structured.summary || memory.content.split("\n").find(Boolean) || "Captured activity";
}

export function TodayView({ onAddNote, onOpenMemory, onOpenFollowUps }: {
  onAddNote: () => void;
  onOpenMemory: (id: number) => void;
  onOpenFollowUps: () => void;
}) {
  const { memories, actions, refreshActions } = useMemento();
  const preview = !("__TAURI_INTERNALS__" in window) && new URLSearchParams(window.location.search).has("demo");
  const sourceMemories = preview ? previewMemories : memories;
  const visibleActions = preview ? previewActions : actions;
  const today = useMemo(() => sourceMemories
    .filter((memory) => memory.createdAt >= startOfToday())
    .sort((a, b) => b.createdAt - a.createdAt), [sourceMemories]);

  const date = new Intl.DateTimeFormat(undefined, { weekday: "long", day: "numeric", month: "long" }).format(new Date());

  async function completeAction(id: number) {
    await api.setActionItemStatus(id, "resolved");
    await refreshActions();
  }

  return (
    <div className="workspace-page daybook">
      <div className="workspace-scroll">
        <section className="workspace-content">
          <header className="workspace-header">
            <div>
              <h1 className="workspace-title">Today</h1>
              <p className="workspace-subtitle">{date}</p>
            </div>
            <div className="flex items-center gap-2">
              <Button onClick={onAddNote}><StickyNoteIcon /> Add note</Button>
            </div>
          </header>

          {preview ? null : <DaySummary onOpenMemory={onOpenMemory} />}

          {visibleActions.length > 0 ? (
            <section aria-labelledby="follow-ups-title" className="border-b py-6">
              <div className="flex items-baseline justify-between gap-4">
                <h2 id="follow-ups-title" className="workspace-section-heading">
                  Follow-ups <span className="font-normal text-muted-foreground">· {visibleActions.length} open</span>
                </h2>
                <button className="text-xs font-medium text-primary hover:underline" onClick={onOpenFollowUps}>
                  {visibleActions.length > 3 ? `View all ${visibleActions.length}` : "View all"}
                </button>
              </div>
              <ul className="mt-3 space-y-1">
                {visibleActions.slice(0, 3).map((action) => (
                  <li key={action.id} className="group flex items-start gap-3 py-2">
                    <button type="button" aria-label={`Complete: ${action.content}`} onClick={() => completeAction(action.id)} className="mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-[5px] border text-transparent hover:border-primary hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40">
                      <CheckIcon className="size-3.5" />
                    </button>
                    <p className="min-w-0 flex-1 text-sm leading-5">{action.content}</p>
                    {action.remindAt ? <time className="shrink-0 text-xs text-muted-foreground">{formatTime(action.remindAt)}</time> : null}
                  </li>
                ))}
              </ul>
            </section>
          ) : null}

          <div className="mt-7">
            {today.length === 0 ? (
              <ol className="daybook-timeline min-h-72 pt-4">
                <li className="daybook-now"><span>Now</span><i /></li>
                <li className="relative py-9 pl-9">
                  <span className="daybook-node top-8"><CircleIcon /></span>
                  <h2 className="text-sm font-semibold">Your day is just getting started</h2>
                  <p className="mt-2 max-w-lg text-sm leading-6 text-muted-foreground">
                    Activity, meetings, and notes will appear along this timeline while Memento runs.
                  </p>
                  <Button variant="outline" className="mt-4" onClick={onAddNote}>Add the first note</Button>
                </li>
              </ol>
            ) : (
              <ol className="daybook-timeline">
                <li className="daybook-now">
                  <span>Now</span><i />
                </li>
                {today.map((memory) => {
                  const Icon = sourceIcon(memory);
                  return (
                    <li key={memory.id} className="daybook-entry">
                      <time className="daybook-time">{formatTime(memory.createdAt)}</time>
                      <span className="daybook-node"><Icon /></span>
                      <button className="daybook-entry-content" onClick={() => onOpenMemory(memory.id)}>
                        <span className="flex items-center gap-2">
                          <strong>{memory.title}</strong>
                          {memory.isMeeting ? <span className="daybook-kind">Meeting</span> : null}
                        </span>
                        <span className="daybook-summary">{summary(memory)}</span>
                        <span className="daybook-meta">{memory.app || (memory.source === "note" ? "Memento note" : memory.source)}</span>
                      </button>
                    </li>
                  );
                })}
              </ol>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
