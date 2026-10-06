import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangleIcon, BanIcon, CheckCircle2Icon, CircleDashedIcon, PauseCircleIcon, ShieldAlertIcon } from "lucide-react";
import { AppIcon, useAppIcons } from "@/components/app-icon";
import { timeAgo } from "@/lib/format";
import { healthApi, SPEED_CHOICES, type CaptureHealth, type HealthState, type RecentDecision } from "@/lib/health";

const nativeRuntime = "__TAURI_INTERNALS__" in window;
/** Light fallback in case an event is missed; only while the section is on screen. */
const REFETCH_MS = 15000;

const STATE_ICON: Record<HealthState, typeof CheckCircle2Icon> = {
  capturing: CheckCircle2Icon,
  paused: PauseCircleIcon,
  "needs-permission": ShieldAlertIcon,
  idle: CircleDashedIcon,
  "not-supported": BanIcon,
  unreadable: AlertTriangleIcon,
  excluded: BanIcon,
};

const PREVIEW: CaptureHealth = {
  state: "capturing", headline: "Capturing Slack", trusted: true, paused: false,
  frontmost: { name: "Slack", bundleId: "com.tinyspeck.slackmacgap" },
  lastDecision: null, lastDecisionAt: null, lastCaptureAt: Date.now() - 45000, consecutiveUnreadable: 0,
  recent: [
    { appName: "Finder", bundleId: "com.apple.finder", reason: "systemApp", label: "System app", explanation: "System screens like Finder and System Settings are never captured.", hint: null, at: Date.now() - 120000, count: 3, tunable: false },
    { appName: "Slack", bundleId: "com.tinyspeck.slackmacgap", reason: "captured", label: "Saved", explanation: "A new memory was saved from this screen.", hint: null, at: Date.now() - 45000, count: 6, tunable: true },
  ],
};

function speedValue(ms: number | undefined): string {
  return ms === undefined ? "default" : String(ms);
}

/** Live answer to "is capture working, and why wasn't that captured?". */
export function CaptureHealthSection() {
  const [health, setHealth] = useState<CaptureHealth | null>(nativeRuntime ? null : PREVIEW);
  const [intervals, setIntervals] = useState<Record<string, number>>({});
  const [error, setError] = useState<string | null>(null);
  const [visible, setVisible] = useState(true);
  const ref = useRef<HTMLElement>(null);

  // Only work while the section can actually be seen.
  useEffect(() => {
    const node = ref.current;
    if (!node || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(([entry]) => setVisible(entry.isIntersecting));
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  const refresh = useCallback(async () => {
    if (!nativeRuntime) return;
    try {
      const [next, saved] = await Promise.all([healthApi.get(), healthApi.intervals()]);
      setHealth(next); setIntervals(saved); setError(null);
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  }, []);

  useEffect(() => {
    if (!nativeRuntime || !visible) return;
    void refresh();
    const stop = healthApi.onChange(setHealth);
    const timer = window.setInterval(() => { if (document.visibilityState === "visible") void refresh(); }, REFETCH_MS);
    return () => { stop(); window.clearInterval(timer); };
  }, [visible, refresh]);

  const tunable = useMemo(() => {
    const seen = new Map<string, string>();
    for (const item of health?.recent ?? []) {
      if (item.bundleId && item.tunable && !seen.has(item.bundleId.toLowerCase())) seen.set(item.bundleId.toLowerCase(), item.appName);
    }
    for (const bundle of Object.keys(intervals)) if (!seen.has(bundle.toLowerCase())) seen.set(bundle.toLowerCase(), bundle);
    return [...seen.entries()].map(([bundleId, name]) => ({ bundleId, name }));
  }, [health, intervals]);
  const icons = useAppIcons(useMemo(() => [...tunable.map((app) => app.bundleId), ...(health?.recent ?? []).flatMap((item) => item.bundleId ? [item.bundleId] : [])], [tunable, health]));

  const lookup = (bundleId: string) => Object.entries(intervals).find(([key]) => key.toLowerCase() === bundleId.toLowerCase())?.[1];

  async function changeSpeed(bundleId: string, raw: string) {
    const ms = raw === "default" ? null : Number(raw);
    const before = intervals;
    const key = Object.keys(intervals).find((item) => item.toLowerCase() === bundleId.toLowerCase()) ?? bundleId;
    const optimistic = { ...intervals };
    if (ms === null) delete optimistic[key]; else optimistic[key] = ms;
    setIntervals(optimistic);
    if (!nativeRuntime) return;
    try { setIntervals(await healthApi.setInterval(bundleId, ms)); setError(null); }
    catch (cause) { setIntervals(before); setError(cause instanceof Error ? cause.message : String(cause)); }
  }

  const StateIcon = health ? STATE_ICON[health.state] : CircleDashedIcon;
  const needsAttention = health?.state === "needs-permission" || health?.state === "unreadable";
  const recent = health?.recent ?? [];

  return <section ref={ref} aria-labelledby="capture-health-title" className="settings-section">
    <div className="mb-3"><h2 id="capture-health-title" className="text-sm font-semibold">Capture health</h2><p className="mt-1 text-xs leading-5 text-muted-foreground">See what Memento is doing right now, and why something wasn’t captured.</p></div>
    <div className="rounded-xl border bg-muted/35 p-4 shadow-sm">
      <div role="status" aria-live="polite" className="flex items-start gap-3">
        <StateIcon aria-hidden="true" className={needsAttention ? "mt-0.5 size-5 shrink-0 text-amber-600" : "mt-0.5 size-5 shrink-0 text-muted-foreground"} />
        <div className="min-w-0">
          <p className="text-sm font-medium">{health ? health.headline : "Checking…"}</p>
          <p className="mt-0.5 text-xs text-muted-foreground">{health?.lastCaptureAt ? `Last saved ${timeAgo(health.lastCaptureAt)}` : "Nothing saved yet this session"}</p>
          {health?.lastDecision && health.state !== "capturing" ? <p className="mt-1 text-xs leading-5 text-muted-foreground">{health.lastDecision.explanation}{health.lastDecision.hint ? ` ${health.lastDecision.hint}` : ""}</p> : null}
        </div>
      </div>
      {error ? <p role="alert" className="mt-3 text-xs text-destructive">{error}</p> : null}

      <div className="mt-5">
        <h3 className="text-sm font-medium">Recent decisions</h3>
        <p className="mt-1 text-xs text-muted-foreground">Apps you were in recently, and what Memento did with them.</p>
        <div className="mt-2 max-h-64 overflow-y-auto rounded-lg border bg-background">
          {recent.length === 0 ? <p className="px-4 py-5 text-center text-xs text-muted-foreground">Nothing yet. Switch to another app and it will show up here.</p> : <ul className="divide-y" aria-label="Recent capture decisions">
            {recent.map((item: RecentDecision) => <li key={`${item.bundleId ?? item.appName}:${item.reason}`} className="flex items-start gap-3 px-3 py-2.5">
              <AppIcon src={item.bundleId ? icons[item.bundleId] : undefined} />
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm"><span>{item.appName}</span> <span className="text-muted-foreground">· {item.label}</span></p>
                <p className="text-xs leading-5 text-muted-foreground">{item.explanation}{item.hint ? ` ${item.hint}` : ""}</p>
              </div>
              <span className="shrink-0 text-xs tabular-nums text-muted-foreground">{timeAgo(item.at)}</span>
            </li>)}
          </ul>}
        </div>
      </div>

      {tunable.length > 0 ? <div className="mt-5">
        <h3 className="text-sm font-medium">Capture speed</h3>
        <p className="mt-1 text-xs text-muted-foreground">How often each app is read while you stay in it. Default follows your general setting.</p>
        <ul className="mt-2 divide-y rounded-lg border bg-background" aria-label="Capture speed by app">
          {tunable.map((app) => {
            const current = lookup(app.bundleId);
            const custom = current !== undefined && !SPEED_CHOICES.some((choice) => choice.value === current);
            return <li key={app.bundleId} className="flex min-h-11 items-center gap-3 px-3">
              <AppIcon src={icons[app.bundleId]} />
              <span className="min-w-0 flex-1 truncate text-sm">{app.name}</span>
              <select value={speedValue(current)} onChange={(event) => void changeSpeed(app.bundleId, event.target.value)} aria-label={`Capture speed for ${app.name}`} className="h-8 rounded-md border bg-background px-2 text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                {SPEED_CHOICES.map((choice) => <option key={choice.label} value={speedValue(choice.value ?? undefined)}>{choice.label}</option>)}
                {custom ? <option value={String(current)}>{`Custom (${Math.round((current ?? 0) / 1000)}s)`}</option> : null}
              </select>
            </li>;
          })}
        </ul>
      </div> : null}
    </div>
  </section>;
}
