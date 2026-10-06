import { useCallback, useEffect, useRef, useState } from "react";
import {
  AlertTriangleIcon,
  CheckIcon,
  ChevronDownIcon,
  CircleCheckIcon,
  CircleSlashIcon,
  EyeOffIcon,
  MessageCircleIcon,
  MicIcon,
  PauseIcon,
  PlayIcon,
  ShieldAlertIcon,
  SquareIcon,
  ZapOffIcon,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import type { CaptureHealth, HealthState } from "@/lib/health";
import { summaryApi, toYmd } from "@/lib/summary";
import type { ActionItem, RecordingMode } from "@/lib/types";
import { cn } from "@/lib/utils";
import {
  dueLabel,
  formatElapsed,
  formatTime,
  pickFollowUps,
  timeAgo,
  trayApi,
  type ActiveRecording,
  type TrayStatus,
} from "@/lib/tray";

const EVENTS = [
  "tray-panel-shown",
  "capture-health",
  "pause-changed",
  "memory-new",
  "recording-started",
  "recording-stopped",
  "recording-auto-stopped",
  "memories-enriched",
] as const;

const PAUSE_CHOICES: { label: string; min?: number }[] = [
  { label: "5 minutes", min: 5 },
  { label: "10 minutes", min: 10 },
  { label: "30 minutes", min: 30 },
  { label: "1 hour", min: 60 },
  { label: "Until I resume" },
];

const STATE_VIEW: Record<HealthState, { icon: typeof CheckIcon; label: string; tone: string }> = {
  capturing: { icon: CircleCheckIcon, label: "Capturing", tone: "text-success" },
  paused: { icon: PauseIcon, label: "Paused", tone: "text-warning" },
  "needs-permission": { icon: ShieldAlertIcon, label: "Needs permission", tone: "text-warning" },
  idle: { icon: CircleSlashIcon, label: "Idle", tone: "text-muted-foreground" },
  "not-supported": { icon: ZapOffIcon, label: "Not supported here", tone: "text-muted-foreground" },
  unreadable: { icon: AlertTriangleIcon, label: "Couldn't read this screen", tone: "text-warning" },
  excluded: { icon: EyeOffIcon, label: "Excluded", tone: "text-muted-foreground" },
};

/** "Capturing Slack" / "Capture is paused" already say the state; do not repeat the word. */
function subtitle(label: string, health: CaptureHealth | null): string {
  const headline = health?.headline.toLowerCase() ?? "";
  const parts = [
    headline.includes(label.toLowerCase()) ? "" : label,
    health?.lastCaptureAt ? `Last saved ${timeAgo(health.lastCaptureAt)}` : "",
  ];
  return parts.filter(Boolean).join(" · ");
}

function useElapsed(startedMs: number | null) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (startedMs == null) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [startedMs]);
  return startedMs == null ? 0 : now - startedMs;
}

export default function TrayPanel() {
  const [health, setHealth] = useState<CaptureHealth | null>(null);
  const [status, setStatus] = useState<TrayStatus | null>(null);
  const [recording, setRecording] = useState<ActiveRecording | null>(null);
  const [followUps, setFollowUps] = useState<ActionItem[]>([]);
  const [openCount, setOpenCount] = useState(0);
  // get_status.memoryCount is the all-time total; the footer says "Today".
  const [todayCount, setTodayCount] = useState<number | null>(null);
  const [note, setNote] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [, setTick] = useState(0);
  const noteRef = useRef<HTMLTextAreaElement>(null);
  const menuOpenRef = useRef(false);
  const savedTimer = useRef<number | undefined>(undefined);
  menuOpenRef.current = menuOpen;

  const elapsed = useElapsed(recording?.startedMs ?? null);

  const refresh = useCallback(async () => {
    const [h, s, r, items, today] = await Promise.allSettled([
      trayApi.health(),
      trayApi.status(),
      trayApi.recordingStatus(),
      trayApi.openFollowUps(),
      summaryApi.daySummary(toYmd(new Date()), false),
    ]);
    if (today.status === "fulfilled") setTodayCount(today.value.memoryCount);
    if (h.status === "fulfilled") setHealth(h.value);
    if (s.status === "fulfilled") setStatus(s.value);
    if (r.status === "fulfilled") setRecording(r.value ?? null);
    if (items.status === "fulfilled") {
      setOpenCount(items.value.length);
      setFollowUps(pickFollowUps(items.value));
    }
    setTick((n) => n + 1); // re-render relative times
  }, []);

  // Transparent webview: the rounded wrapper draws the surface.
  useEffect(() => {
    const els = [document.documentElement, document.body];
    const prev = els.map((el) => el.style.background);
    els.forEach((el) => (el.style.background = "transparent"));
    return () => els.forEach((el, i) => (el.style.background = prev[i]));
  }, []);

  useEffect(() => {
    let cancelled = false;
    let debounce: number | undefined;
    const unlisteners: Array<() => void> = [];
    const schedule = () => {
      window.clearTimeout(debounce);
      debounce = window.setTimeout(() => void refresh(), 200);
    };
    void refresh();
    for (const name of EVENTS) {
      void listen(name, () => {
        if (name === "tray-panel-shown") {
          setMenuOpen(false);
          setNotice(null);
          setError(null);
          void refresh();
          requestAnimationFrame(() => noteRef.current?.focus());
        } else {
          schedule();
        }
      }).then((fn) => {
        if (cancelled) fn();
        else unlisteners.push(fn);
      });
    }
    noteRef.current?.focus();
    return () => {
      cancelled = true;
      window.clearTimeout(debounce);
      window.clearTimeout(savedTimer.current);
      unlisteners.forEach((fn) => fn());
    };
  }, [refresh]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      if (menuOpenRef.current) setMenuOpen(false);
      else void trayApi.hide().catch(() => {});
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, []);

  const run = useCallback(
    async (fn: () => Promise<unknown>) => {
      setBusy(true);
      setError(null);
      try {
        await fn();
        await refresh();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    [refresh],
  );

  const saveNote = useCallback(async () => {
    const text = note.trim();
    if (!text || saving) return;
    setSaving(true);
    setError(null);
    try {
      await trayApi.insertNote(text);
      setNote("");
      setSaved(true);
      window.clearTimeout(savedTimer.current);
      savedTimer.current = window.setTimeout(() => setSaved(false), 1800);
      void refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }, [note, saving, refresh]);

  const startRecording = () =>
    run(async () => {
      setNotice(null);
      const settings = await trayApi.settings();
      if (!settings.recordMic && !settings.recordSystem) {
        throw new Error("Turn on the microphone or system audio in Settings first.");
      }
      let mode: RecordingMode =
        settings.recordMic && settings.recordSystem ? "both" : settings.recordSystem ? "system" : "mic";
      const permissions = await trayApi.permissions();
      if (mode !== "mic" && permissions.screenRecording !== "granted") {
        if (mode === "system") {
          throw new Error("Recording other people needs Screen Recording access. Allow it in Settings.");
        }
        mode = "mic";
        setNotice("Recording your microphone only. Allow Screen Recording in Settings to capture the other side too.");
      }
      if ((mode === "mic" || mode === "both") && permissions.microphone !== "granted") {
        await trayApi.requestPermission("microphone");
      }
      await trayApi.startRecording(mode, "meeting");
    });

  const paused = health?.paused ?? status?.paused ?? false;
  const state: HealthState = health?.state ?? "idle";
  const view = STATE_VIEW[state];
  const StateIcon = view.icon;
  const needsPermission = state === "needs-permission" || status?.trusted === false;
  const resumeAt = paused && status?.pauseUntil ? status.pauseUntil : null;

  return (
    <div className="h-screen w-screen overflow-hidden p-0">
      <div className="flex h-full flex-col overflow-hidden rounded-2xl border bg-background/95 text-foreground shadow-2xl backdrop-blur-xl">
        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto overflow-x-hidden px-3 pt-3 pb-2">
          {/* 1. Status */}
          <section aria-label="Capture status" aria-live="polite" className="space-y-2">
            {recording ? (
              <div className="flex items-center gap-2.5 rounded-lg bg-destructive/10 px-3 py-2">
                <span className="relative flex size-2.5 shrink-0" aria-hidden="true">
                  <span className="absolute inline-flex size-full animate-ping rounded-full bg-destructive opacity-60 motion-reduce:animate-none" />
                  <span className="relative inline-flex size-2.5 rounded-full bg-destructive" />
                </span>
                <div className="min-w-0 flex-1">
                  <div className="text-sm font-semibold text-destructive">Recording</div>
                  <div className="text-xs text-muted-foreground">Capturing audio for a meeting</div>
                </div>
                <span className="font-mono text-base font-semibold tabular-nums" aria-label={`Recording time ${formatElapsed(elapsed)}`}>
                  {formatElapsed(elapsed)}
                </span>
              </div>
            ) : (
              <div className="flex items-start gap-2.5 px-1">
                <StateIcon className={cn("mt-0.5 size-4 shrink-0", view.tone)} aria-hidden="true" />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-semibold" title={health?.headline}>
                    {health?.headline ?? "Checking…"}
                  </div>
                  <div className="truncate text-xs text-muted-foreground">
                    {subtitle(view.label, health)}
                  </div>
                </div>
              </div>
            )}
            {needsPermission && (
              <Button className="w-full" onClick={() => run(() => trayApi.requestAccessibility())}>
                <ShieldAlertIcon data-icon="inline-start" /> Allow Accessibility
              </Button>
            )}
          </section>

          {/* Ask — the same floating panel as ⌘⇧L, one click from the menu bar. */}
          <button
            type="button"
            onClick={() => { void trayApi.askYourDay().then(() => trayApi.hide()); }}
            className="flex h-9 w-full items-center gap-2 rounded-lg border bg-background/60 px-3 text-left text-sm text-muted-foreground outline-none transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50"
            aria-label="Ask your day, keyboard shortcut Command Shift L"
          >
            <MessageCircleIcon className="size-4 shrink-0" aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate">Ask about your day…</span>
            <kbd className="font-sans text-[10px]" aria-hidden="true">⌘⇧L</kbd>
          </button>

          {/* 2. Controls */}
          <section aria-label="Controls" className="space-y-1.5">
            <div className="grid grid-cols-2 gap-2">
              <div className="relative">
                {paused ? (
                  <Button
                    variant="outline"
                    className="w-full"
                    disabled={busy}
                    onClick={() => run(() => trayApi.setPaused(false))}
                  >
                    <PlayIcon data-icon="inline-start" /> Resume
                  </Button>
                ) : (
                  <Button
                    variant="outline"
                    className="w-full"
                    aria-haspopup="menu"
                    aria-expanded={menuOpen}
                    disabled={busy}
                    onClick={() => setMenuOpen((o) => !o)}
                  >
                    <PauseIcon data-icon="inline-start" /> Pause
                    <ChevronDownIcon data-icon="inline-end" />
                  </Button>
                )}
                {menuOpen && !paused && (
                  <PauseMenu
                    onClose={() => setMenuOpen(false)}
                    onPick={(min) => {
                      setMenuOpen(false);
                      void run(() => trayApi.setPaused(true, min));
                    }}
                  />
                )}
              </div>
              {recording ? (
                <Button variant="destructive" disabled={busy} onClick={() => run(() => trayApi.stopRecording())}>
                  <SquareIcon data-icon="inline-start" /> Stop recording
                </Button>
              ) : (
                <Button variant="outline" disabled={busy} onClick={startRecording}>
                  <MicIcon data-icon="inline-start" /> Start recording
                </Button>
              )}
            </div>
            {paused && (
              <p className="px-1 text-xs text-muted-foreground">
                {resumeAt ? `Resumes at ${formatTime(resumeAt)}` : "Paused until you resume"}
              </p>
            )}
            {notice && <p className="px-1 text-xs text-muted-foreground">{notice}</p>}
            {error && (
              <p role="alert" className="px-1 text-xs text-destructive">
                {error}
              </p>
            )}
          </section>

          {/* 3. Quick note */}
          <section aria-label="Quick note" className="space-y-1.5">
            <label htmlFor="tray-note" className="sr-only">
              Quick note
            </label>
            <textarea
              id="tray-note"
              ref={noteRef}
              value={note}
              rows={3}
              onChange={(e) => setNote(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  void saveNote();
                }
              }}
              placeholder="Jot something down…"
              className="block w-full resize-none rounded-lg border border-input bg-input/30 px-2.5 py-2 text-sm outline-none transition-colors placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
            />
            <div className="flex items-center justify-between gap-2">
              <span role="status" className={cn("flex items-center gap-1 text-xs", saved ? "text-success" : "text-muted-foreground")}>
                {saved ? (
                  <>
                    <CheckIcon className="size-3.5" aria-hidden="true" /> Saved
                  </>
                ) : (
                  "⌘Enter to save"
                )}
              </span>
              <Button size="sm" onClick={saveNote} disabled={!note.trim() || saving}>
                {saving ? "Saving…" : "Save note"}
              </Button>
            </div>
          </section>

          {/* 4. Follow-ups */}
          <section aria-labelledby="tray-followups" className="space-y-1">
            <div className="flex items-center justify-between px-1">
              <h2 id="tray-followups" className="text-xs font-medium text-muted-foreground">
                Follow-ups · {openCount} open
              </h2>
              <Button variant="link" size="xs" className="h-5 px-0" onClick={() => void trayApi.openMainView("actions")}>
                View all
              </Button>
            </div>
            {followUps.length === 0 ? (
              <p className="rounded-lg px-1 py-2 text-xs text-muted-foreground">Nothing to follow up on right now.</p>
            ) : (
              <ul className="space-y-0.5">
                {followUps.map((item) => {
                  const due = dueLabel(item.remindAt);
                  const overdue = item.remindAt != null && item.remindAt <= Date.now();
                  return (
                    <li key={item.id} className="flex items-start gap-2.5 rounded-lg px-1 py-1.5 hover:bg-muted/50">
                      <Checkbox
                        className="mt-0.5"
                        checked={false}
                        aria-label={`Mark done: ${item.content}`}
                        onCheckedChange={() => {
                          setFollowUps((prev) => prev.filter((f) => f.id !== item.id));
                          setOpenCount((n) => Math.max(0, n - 1));
                          void run(() => trayApi.resolveFollowUp(item.id));
                        }}
                      />
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-sm" title={item.content}>
                          {item.content}
                        </div>
                        {due && (
                          <div className={cn("truncate text-xs", overdue ? "text-warning" : "text-muted-foreground")}>{due}</div>
                        )}
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </section>
        </div>

        {/* 5. Footer */}
        <footer className="flex items-center gap-2 border-t px-3 py-2">
          <Button size="sm" onClick={() => void trayApi.openMainView("today")}>
            Open Memento
          </Button>
          <Button size="sm" variant="ghost" onClick={() => void trayApi.openMainView("settings")}>
            Settings
          </Button>
          <span className="ml-auto truncate text-xs text-muted-foreground">
            {todayCount !== null ? `Today: ${todayCount} ${todayCount === 1 ? "memory" : "memories"}` : ""}
          </span>
        </footer>
      </div>
    </div>
  );
}

function PauseMenu({ onPick, onClose }: { onPick: (min?: number) => void; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.querySelector<HTMLButtonElement>("button")?.focus();
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [onClose]);

  const move = (dir: 1 | -1) => {
    const items = Array.from(ref.current?.querySelectorAll<HTMLButtonElement>("button") ?? []);
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    items[(i + dir + items.length) % items.length]?.focus();
  };

  return (
    <div
      ref={ref}
      role="menu"
      aria-label="Pause capture for"
      onKeyDown={(e) => {
        if (e.key === "ArrowDown") (e.preventDefault(), move(1));
        else if (e.key === "ArrowUp") (e.preventDefault(), move(-1));
      }}
      className="absolute top-full left-0 z-20 mt-1 w-44 rounded-lg border bg-popover p-1 text-popover-foreground shadow-lg"
    >
      {PAUSE_CHOICES.map((c) => (
        <button
          key={c.label}
          role="menuitem"
          type="button"
          onClick={() => onPick(c.min)}
          className="flex w-full items-center rounded-md px-2 py-1.5 text-left text-sm outline-none hover:bg-muted focus-visible:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50"
        >
          {c.label}
        </button>
      ))}
    </div>
  );
}
