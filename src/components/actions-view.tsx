import { useEffect, useRef, useState } from "react";
import {
  AlarmClockIcon,
  CheckIcon,
  CircleXIcon,
  ClockIcon,
  PencilIcon,
  SparklesIcon,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/api";
import { formatTime, timeAgo } from "@/lib/format";
import { useMemento } from "@/lib/store";
import { summaryApi, type SnoozePreset } from "@/lib/summary";
import type { ActionItem, DueReminders } from "@/lib/types";

const MAX_TEXT = 500;

/** "Today 5:00 PM", "Tomorrow 9:00 AM", "Mon 12 Oct, 9:00 AM". */
function formatDue(ms: number): string {
  const when = new Date(ms);
  const now = new Date();
  const dayDiff = Math.round(
    (new Date(when.getFullYear(), when.getMonth(), when.getDate()).getTime() -
      new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()) / 86_400_000,
  );
  if (dayDiff === 0) return `Today ${formatTime(ms)}`;
  if (dayDiff === 1) return `Tomorrow ${formatTime(ms)}`;
  if (dayDiff === -1) return `Yesterday ${formatTime(ms)}`;
  const day = new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short" }).format(when);
  return `${day}, ${formatTime(ms)}`;
}

function toLocalInput(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function SnoozeMenu({
  item,
  onSnoozed,
}: {
  item: ActionItem;
  onSnoozed: () => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [presets, setPresets] = useState<SnoozePreset[]>([]);
  const [picking, setPicking] = useState(false);
  const [custom, setCustom] = useState("");
  const [error, setError] = useState<string | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);

  function close(returnFocus = true) {
    setOpen(false);
    setPicking(false);
    setError(null);
    if (returnFocus) trigger.current?.focus();
  }

  useEffect(() => {
    if (!open) return;
    summaryApi.snoozePresets().then(setPresets).catch(() => setPresets([]));
    const onPointer = (event: PointerEvent) => {
      if (root.current && !root.current.contains(event.target as Node)) close(false);
    };
    document.addEventListener("pointerdown", onPointer);
    return () => document.removeEventListener("pointerdown", onPointer);
  }, [open]);

  useEffect(() => {
    if (open && presets.length > 0) root.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
  }, [open, presets.length]);

  async function snooze(untilMs: number) {
    setError(null);
    try {
      await summaryApi.snoozeActionItem(item.id, untilMs);
      await onSnoozed();
      close();
    } catch (e) {
      setError(String(e));
    }
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (event.key === "Escape") {
      event.stopPropagation();
      close();
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const items = Array.from(root.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? []);
    if (items.length === 0) return;
    event.preventDefault();
    const at = items.indexOf(document.activeElement as HTMLElement);
    const next = event.key === "ArrowDown" ? (at + 1) % items.length : (at - 1 + items.length) % items.length;
    items[next].focus();
  }

  return (
    <div ref={root} className="relative" onKeyDown={onKeyDown}>
      <Button
        ref={trigger}
        variant="ghost"
        size="sm"
        className="px-2 text-xs text-muted-foreground"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Snooze: ${item.content}`}
        onClick={() => (open ? close() : setOpen(true))}
      >
        <ClockIcon data-icon="inline-start" className="size-3.5" />
        Snooze
      </Button>
      {open ? (
        <div role="menu" aria-label="Snooze until" className="absolute right-0 top-full z-20 mt-1 w-72 rounded-lg border bg-popover p-1 text-popover-foreground shadow-lg">
          {presets.map((preset) => (
            <button
              key={preset.id}
              type="button"
              role="menuitem"
              className="flex w-full items-baseline justify-between gap-3 rounded-md px-2 py-1.5 text-left text-sm outline-none hover:bg-muted focus-visible:bg-muted"
              onClick={() => snooze(preset.untilMs)}
            >
              <span className="whitespace-nowrap">{preset.label.split(",")[0]}</span>
              <span className="whitespace-nowrap text-xs text-muted-foreground">{formatDue(preset.untilMs)}</span>
            </button>
          ))}
          <button
            type="button"
            role="menuitem"
            className="flex w-full rounded-md px-2 py-1.5 text-left text-sm outline-none hover:bg-muted focus-visible:bg-muted"
            aria-expanded={picking}
            onClick={() => {
              setPicking(true);
              setCustom(toLocalInput(Date.now() + 60 * 60 * 1000));
            }}
          >
            Pick date &amp; time…
          </button>
          {picking ? (
            <form
              className="flex items-center gap-1.5 px-2 pb-1.5 pt-1"
              onSubmit={(event) => {
                event.preventDefault();
                const ms = new Date(custom).getTime();
                if (Number.isNaN(ms)) setError("Choose a date and time.");
                else snooze(ms);
              }}
            >
              <Input
                type="datetime-local"
                aria-label="Snooze until date and time"
                value={custom}
                min={toLocalInput(Date.now())}
                onChange={(event) => setCustom(event.target.value)}
                className="h-7 text-xs md:text-xs"
                autoFocus
              />
              <Button type="submit" size="sm">Set</Button>
            </form>
          ) : null}
          {error ? <p role="alert" className="px-2 pb-1.5 text-xs text-destructive">{error}</p> : null}
        </div>
      ) : null}
    </div>
  );
}

function NudgeBanner({
  reminders,
  onDismiss,
}: {
  reminders: DueReminders;
  onDismiss: () => void;
}) {
  return (
    <div className="mb-5 border-y border-warning/30 bg-warning/10 px-1 py-3">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <div className="flex items-center gap-2 text-sm font-medium">
            <AlarmClockIcon className="size-4" />
            {reminders.dnd ? "Reminders held while Do Not Disturb is on" : "A nudge for you"}
            <Badge className="bg-warning/20 text-warning">
              {reminders.total} item
              {reminders.total === 1 ? "" : "s"} — one summary, not a pile
            </Badge>
          </div>
          <div className="flex flex-col gap-0.5 text-xs text-muted-foreground">
            {reminders.batches.map((batch) => (
              <div key={batch.id}>
                <span className="font-medium text-foreground/80">{batch.label}:</span>{" "}
                {batch.items.map((i) => i.content).join(" · ")}
              </div>
            ))}
          </div>
        </div>
        <Button
          variant="outline"
          size="sm"
          className="shrink-0"
          onClick={onDismiss}
        >
          Dismiss
        </Button>
      </div>
    </div>
  );
}

export function ActionsView() {
  const { actions, refreshActions, markActionsRead, dueReminders, dismissReminders } =
    useMemento();
  const [done, setDone] = useState(false);
  const [doneItems, setDoneItems] = useState<ActionItem[]>([]);
  const [summary, setSummary] = useState<string | null>(null);
  const [summarizing, setSummarizing] = useState(false);
  const [busy, setBusy] = useState<number | null>(null);
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const [editError, setEditError] = useState<string | null>(null);

  useEffect(() => {
    markActionsRead().then(() => refreshActions());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function complete(id: number) {
    await api.setActionItemStatus(id, "resolved");
    await refreshActions();
  }

  async function dismiss(id: number) {
    await api.setActionItemStatus(id, "dismissed");
    await refreshActions();
  }

  async function remindInAnHour(item: ActionItem) {
    setBusy(item.id);
    try {
      const remindAt = Date.now() + 60 * 60 * 1000;
      await api.updateActionItem(item.id, { remindAt });
      await refreshActions();
    } finally {
      setBusy(null);
    }
  }

  function startEdit(item: ActionItem) {
    setEditing(item.id);
    setDraft(item.content);
    setEditError(null);
  }

  async function saveEdit(item: ActionItem) {
    if (draft.trim() === item.content) {
      setEditing(null);
      return;
    }
    try {
      await summaryApi.editActionItemText(item.id, draft);
      setEditing(null);
      await refreshActions();
    } catch (e) {
      setEditError(String(e));
    }
  }

  async function summarize() {
    setSummarizing(true);
    try {
      const { result } = await api.agentRun("open-loops", "Summarize my open loops");
      setSummary(result);
    } catch (e) {
      console.error(e);
    } finally {
      setSummarizing(false);
    }
  }

  async function showDone() {
    setDone(true);
    setDoneItems(await api.listActionItems("resolved"));
  }

  const list = done ? doneItems : actions;

  return (
    <div className="workspace-page">
      <div className="workspace-content flex min-h-0 flex-1 flex-col overflow-hidden">
      <header className="workspace-header">
        <div>
          <h1 className="workspace-title">Follow-ups</h1>
          <p className="workspace-subtitle">Promises, deadlines, and things you wanted to come back to.</p>
        </div>
        <div className="flex items-center gap-2">
        {done ? (
          <Button variant="ghost" size="sm" onClick={() => setDone(false)}>
            Back to open
          </Button>
        ) : (
          <Button variant="outline" size="sm" onClick={showDone}>
            View done
          </Button>
        )}
        <Button
          variant="outline"
          size="sm"
          onClick={summarize}
          disabled={summarizing}
        >
          <SparklesIcon data-icon="inline-start" />
          {summarizing ? "Summarizing…" : "Local summary"}
        </Button>
        </div>
      </header>

      <div className="workspace-scroll">
      {dueReminders && dueReminders.batches.length > 0 ? (
        <NudgeBanner
          reminders={dueReminders}
          onDismiss={() => dismissReminders([])}
        />
      ) : null}

      {summary ? (
        <div className="mb-3 rounded-lg border bg-muted/40 p-3">
          <pre className="whitespace-pre-wrap text-sm leading-relaxed">
            {summary}
          </pre>
        </div>
      ) : null}

      <div className="min-h-0">
        {list.length === 0 ? (
          <Empty>
            <EmptyHeader>
              <EmptyTitle>
                {done ? "Nothing resolved yet" : "Nothing on your plate"}
              </EmptyTitle>
              <EmptyDescription>
                Action items are picked out of your captured memory and
                meeting transcripts — commitments, follow-ups, and deadlines.
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <div className="flex flex-col">
            {list.map((item) => (
              <div
                key={item.id}
                className={`group flex items-start gap-3 border-b px-1 py-4 transition-colors hover:bg-muted/35 ${
                  item.isUrgent && !done ? "bg-warning/5" : ""
                }`}
              >
                <button
                  type="button"
                  aria-label="Mark resolved"
                  onClick={() => (done ? null : complete(item.id))}
                  className="mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-[4px] border border-input transition-colors hover:bg-muted"
                >
                  {done ? <CheckIcon className="size-3" /> : null}
                </button>
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <div className="flex items-start gap-1.5">
                    {item.isUnread && !done ? (
                      <span
                        aria-label="Unread"
                        className="mt-1.5 size-1.5 shrink-0 rounded-full bg-primary"
                      />
                    ) : null}
                    {editing === item.id ? (
                      <div className="flex min-w-0 flex-1 flex-col gap-1">
                        <Input
                          autoFocus
                          value={draft}
                          maxLength={MAX_TEXT}
                          aria-label="Edit follow-up text"
                          aria-invalid={editError ? true : undefined}
                          onChange={(event) => {
                            setDraft(event.target.value);
                            setEditError(null);
                          }}
                          onKeyDown={(event) => {
                            if (event.key === "Enter") {
                              event.preventDefault();
                              saveEdit(item);
                            } else if (event.key === "Escape") {
                              event.stopPropagation();
                              setEditing(null);
                            }
                          }}
                        />
                        <span className="text-[11px] text-muted-foreground">Enter to save · Esc to cancel</span>
                        {editError ? <span role="alert" className="text-xs text-destructive">{editError}</span> : null}
                      </div>
                    ) : (
                      <span
                        className="text-sm font-medium"
                        onDoubleClick={() => (done ? null : startEdit(item))}
                      >
                        {item.content}
                      </span>
                    )}
                  </div>
                  <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
                    <span>
                      {done ? "Resolved" : "Captured"} {timeAgo(item.createdAt)}
                    </span>
                    {item.isUrgent && !done ? (
                      <Badge className="bg-warning/15 text-warning">Urgent</Badge>
                    ) : null}
                    {item.remindAt && !done ? (
                      <Badge variant="secondary">
                        <AlarmClockIcon className="size-3" aria-hidden="true" />
                        {item.remindAt < Date.now() ? "Was due" : "Due"} {formatDue(item.remindAt)}
                      </Badge>
                    ) : null}
                  </div>
                </div>
                {!done ? (
                  <div className="flex shrink-0 items-center gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      className="px-2 text-xs text-muted-foreground"
                      onClick={() => startEdit(item)}
                      title="Edit"
                      aria-label={`Edit follow-up: ${item.content}`}
                    >
                      <PencilIcon className="size-3.5" />
                    </Button>
                    <SnoozeMenu item={item} onSnoozed={refreshActions} />
                    <Button
                      variant="ghost"
                      size="sm"
                      className="px-2 text-xs text-muted-foreground"
                      disabled={busy === item.id}
                      onClick={() => remindInAnHour(item)}
                    >
                      <AlarmClockIcon data-icon="inline-start" className="size-3.5" />
                      Remind in 1 hour
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="px-2 text-xs text-muted-foreground"
                      onClick={() => dismiss(item.id)}
                      title="Dismiss"
                      aria-label="Dismiss follow-up"
                    >
                      <CircleXIcon className="size-3.5" />
                    </Button>
                  </div>
                ) : null}
              </div>
            ))}
          </div>
        )}
      </div>
      </div>
      </div>
    </div>
  );
}
