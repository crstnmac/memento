import { useEffect, useMemo, useState } from "react";
import { ChevronDownIcon, SparklesIcon } from "lucide-react";

import { formatDuration, formatTime } from "@/lib/format";
import { useMemento } from "@/lib/store";
import {
  addDays,
  fromYmd,
  summaryApi,
  toYmd,
  weekStartOf,
  type PeriodSummary,
} from "@/lib/summary";
import { cn } from "@/lib/utils";

type Mode = "day" | "week";

const shortDate = (d: Date) =>
  new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short" }).format(d);

const hourLabel = (hour: number) =>
  new Date(2000, 0, 1, hour).toLocaleTimeString([], { hour: "numeric" });

/** `step` is how many days/weeks back from now (0 = current). */
function periodStart(mode: Mode, step: number): Date {
  const now = new Date();
  return mode === "day" ? addDays(now, -step) : addDays(weekStartOf(now), -7 * step);
}

function periodTitle(mode: Mode, step: number, start: Date) {
  if (mode === "day") {
    if (step === 0) return "Today so far";
    if (step === 1) return "Yesterday";
    return shortDate(start);
  }
  if (step === 0) return "This week so far";
  if (step === 1) return "Last week";
  return `Week of ${new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" }).format(start)}`;
}

/** "5 memories · 1 meeting · 1 voice note" — the whole story in one line. */
function factLine(summary: PeriodSummary): string {
  const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
  const meetings = summary.meetings.filter((m) => m.kind !== "voice-note").length;
  const notes = summary.meetings.length - meetings;
  return [
    plural(summary.memoryCount, "memory", "memories"),
    meetings > 0 ? plural(meetings, "meeting") : null,
    notes > 0 ? plural(notes, "voice note") : null,
  ].filter(Boolean).join(" · ");
}

function Stat({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-[11px] font-medium text-muted-foreground">{label}</dt>
      <dd className="mt-0.5 text-sm tabular-nums">{children}</dd>
    </div>
  );
}

function Chip({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "h-6 rounded-md px-2 text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring/50",
        active ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-muted hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}

export function DaySummary({ onOpenMemory }: { onOpenMemory: (id: number) => void }) {
  const { memories } = useMemento();
  const [mode, setMode] = useState<Mode>("day");
  const [step, setStep] = useState(0);
  const [summary, setSummary] = useState<PeriodSummary | null>(null);
  const [narrative, setNarrative] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);

  const start = useMemo(() => periodStart(mode, step), [mode, step]);
  const key = toYmd(start);
  const fetchSummary = (withNarrative: boolean) =>
    mode === "day" ? summaryApi.daySummary(key, withNarrative) : summaryApi.weekSummary(key, withNarrative);

  // Numbers first (instant, local); refreshed as new memories arrive.
  const liveCount = step === 0 ? memories.length : 0;
  useEffect(() => {
    let cancelled = false;
    fetchSummary(false)
      .then((next) => {
        if (cancelled) return;
        setSummary(next);
        setFailed(false);
      })
      .catch(() => !cancelled && setFailed(true));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, key, liveCount]);

  // The narrative may take a while (or never come); it never blocks the numbers.
  useEffect(() => {
    let cancelled = false;
    setNarrative(null);
    fetchSummary(true)
      .then((next) => !cancelled && setNarrative(next.narrative))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, key]);

  if (failed && !summary) return null;
  const current = summary && summary.startDate === key && summary.period === mode ? summary : null;
  const text = current?.narrative ?? (current ? narrative : null);
  const title = periodTitle(mode, step, start);
  const when = mode === "day" ? "today" : "this week";
  const empty = current && current.memoryCount === 0 && current.meetings.length === 0;
  const maxDay = Math.max(1, ...(current?.days.map((d) => d.count) ?? [1]));

  return (
    <section aria-labelledby="day-summary-title" className="border-b py-5" aria-busy={!current}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 id="day-summary-title" className="workspace-section-heading">{title}</h2>
        <div role="group" aria-label="Summary period" className="flex items-center gap-0.5">
          <Chip active={mode === "day" && step === 0} onClick={() => { setMode("day"); setStep(0); }}>Today</Chip>
          <Chip active={mode === "day" && step === 1} onClick={() => { setMode("day"); setStep(1); }}>Yesterday</Chip>
          <Chip active={mode === "week" && step === 0} onClick={() => { setMode("week"); setStep(0); }}>This week</Chip>
          <Chip active={mode === "week" && step === 1} onClick={() => { setMode("week"); setStep(1); }}>Last week</Chip>
        </div>
      </div>

      {!current ? (
        <p className="mt-3 text-sm text-muted-foreground">Gathering the day…</p>
      ) : empty ? (
        <p className="mt-3 text-sm text-muted-foreground">
          Nothing was captured {mode === "day" ? (step === 0 ? "yet today" : "that day") : "that week"}.
          {step === 0 ? " It will show up here as Memento runs." : ""}
        </p>
      ) : (
        <div className="mt-3 space-y-4">
          {text ? (
            <div>
              <h3 className="flex items-center gap-1.5 text-[11px] font-medium text-muted-foreground">
                <SparklesIcon className="size-3" aria-hidden="true" />
                {mode === "day" && step === 0 ? "What happened today" : `What happened ${mode === "day" ? "that day" : when}`}
              </h3>
              <p className="mt-1 max-w-2xl text-sm leading-6">{text}</p>
            </div>
          ) : null}

          <p className="text-sm text-muted-foreground tabular-nums">{factLine(current)}</p>

          {current.days.length > 0 ? (
            <ol aria-label="Memories per day" className="grid grid-cols-7 gap-1.5">
              {current.days.map((d) => (
                <li key={d.date} className="text-center">
                  <div className="flex h-8 items-end justify-center" aria-hidden="true">
                    <span className="w-full max-w-6 rounded-sm bg-primary/60" style={{ height: `${Math.max(d.count ? 12 : 2, (d.count / maxDay) * 100)}%` }} />
                  </div>
                  <span className="mt-1 block text-[11px] text-muted-foreground">
                    {new Intl.DateTimeFormat(undefined, { weekday: "short" }).format(fromYmd(d.date))}
                  </span>
                  <span className="block text-xs tabular-nums">{d.count}</span>
                </li>
              ))}
            </ol>
          ) : null}

          {/* Everything secondary lives here so the page stays calm; it is one
              click away and nothing is dropped. */}
          <div>
            <button
              type="button"
              aria-expanded={detailsOpen}
              aria-controls="day-summary-details"
              onClick={() => setDetailsOpen((open) => !open)}
              className="-ml-1 inline-flex items-center gap-1 rounded-md px-1 py-0.5 text-xs font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
            >
              <ChevronDownIcon className={cn("size-3.5 transition-transform", detailsOpen ? "" : "-rotate-90")} aria-hidden="true" />
              {detailsOpen ? "Hide details" : "Show details"}
            </button>
            {detailsOpen ? (
              <div id="day-summary-details" className="mt-3 space-y-4">
                <dl className="grid grid-cols-2 gap-x-6 gap-y-3 sm:grid-cols-3">
                  <Stat label="Active between">
                    {current.firstAt && current.lastAt
                      ? current.period === "day"
                        ? `${formatTime(current.firstAt)} – ${formatTime(current.lastAt)}`
                        : `${shortDate(new Date(current.firstAt))} – ${shortDate(new Date(current.lastAt))}`
                      : "—"}
                  </Stat>
                  <Stat label="Busiest hour">
                    {current.busiestHour ? `${hourLabel(current.busiestHour.hour)} (${current.busiestHour.count})` : "—"}
                  </Stat>
                  <Stat label="Follow-ups">
                    {current.followUps.created} new · {current.followUps.resolved} done · {current.followUps.stillOpen} open
                  </Stat>
                  {current.apps.length > 0 ? (
                    <div className="col-span-2 min-w-0 sm:col-span-3">
                      <dt className="text-[11px] font-medium text-muted-foreground">Where it happened</dt>
                      <dd className="mt-0.5 truncate text-sm">
                        {current.apps.slice(0, 5).map((a) => `${a.app} ${a.count}`).join(" · ")}
                      </dd>
                    </div>
                  ) : null}
                </dl>

                {current.topics.length > 0 || current.meetings.length > 0 ? (
                  <div className="grid gap-x-6 gap-y-3 sm:grid-cols-2">
                    {current.topics.length > 0 ? (
                      <div>
                        <h3 className="text-[11px] font-medium text-muted-foreground">Most active threads</h3>
                        <ul className="mt-1">
                          {current.topics.slice(0, 4).map((t) => (
                            <li key={`${t.memoryId}`}>
                              <button
                                type="button"
                                onClick={() => onOpenMemory(t.memoryId)}
                                className="flex w-full items-baseline justify-between gap-3 rounded-md px-1 py-1 text-left text-sm outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50"
                              >
                                <span className="truncate">{t.title}</span>
                                <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
                                  {t.count} {t.count === 1 ? "update" : "updates"}
                                </span>
                              </button>
                            </li>
                          ))}
                        </ul>
                      </div>
                    ) : null}
                    {current.meetings.length > 0 ? (
                      <div>
                        <h3 className="text-[11px] font-medium text-muted-foreground">Meetings & voice notes</h3>
                        <ul className="mt-1">
                          {current.meetings.slice(0, 4).map((m) => {
                            const detail = `${m.kind === "voice-note" ? "Voice note" : "Meeting"}${m.durationMs ? ` · ${formatDuration(m.durationMs)}` : ""}`;
                            const body = (
                              <>
                                <span className="truncate">{m.title}</span>
                                <span className="shrink-0 text-xs text-muted-foreground">{detail}</span>
                              </>
                            );
                            return (
                              <li key={m.id}>
                                {m.memoryId ? (
                                  <button
                                    type="button"
                                    onClick={() => onOpenMemory(m.memoryId as number)}
                                    className="flex w-full items-baseline justify-between gap-3 rounded-md px-1 py-1 text-left text-sm outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50"
                                  >
                                    {body}
                                  </button>
                                ) : (
                                  <div className="flex items-baseline justify-between gap-3 px-1 py-1 text-sm">{body}</div>
                                )}
                              </li>
                            );
                          })}
                        </ul>
                      </div>
                    ) : null}
                  </div>
                ) : null}
              </div>
            ) : null}
          </div>
        </div>
      )}
    </section>
  );
}
