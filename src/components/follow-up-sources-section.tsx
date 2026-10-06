import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { GlobeIcon, MailIcon, MessageSquareIcon, Mic2Icon, NotebookPenIcon } from "lucide-react";
import { AppIcon, useAppIcons } from "@/components/app-icon";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import type { FollowUpCategory, FollowUpSource } from "@/lib/types";

const nativeRuntime = "__TAURI_INTERNALS__" in window;

const CATEGORIES: { id: FollowUpCategory; label: string; icon: React.ComponentType<{ className?: string }> }[] = [
  { id: "chat", label: "Chat", icon: MessageSquareIcon },
  { id: "email", label: "Email", icon: MailIcon },
  { id: "meeting", label: "Meetings", icon: Mic2Icon },
  { id: "productivity", label: "Notes & tasks", icon: NotebookPenIcon },
];

const PREVIEW: FollowUpSource[] = [
  { id: "slack", name: "Slack", category: "chat", bundleIds: [] },
  { id: "gmail", name: "Gmail", category: "email", bundleIds: [] },
  { id: "meeting", name: "Meetings", category: "meeting", bundleIds: [] },
  { id: "apple-notes", name: "Apple Notes", category: "productivity", bundleIds: [] },
];

/**
 * The include side of capture: Memento only looks for follow-ups in the apps
 * left on here. Every app is on by default; switching one off stops new
 * follow-ups from it without touching what is captured or already saved.
 */
export function FollowUpSourcesSection() {
  const [sources, setSources] = useState<FollowUpSource[]>(nativeRuntime ? [] : PREVIEW);
  const [disabled, setDisabled] = useState<string[]>([]);
  const [loading, setLoading] = useState(nativeRuntime);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!nativeRuntime) return;
    api.getFollowUpSources()
      .then((result) => { setSources(result.sources); setDisabled(result.disabled); })
      .catch((cause) => setError(cause instanceof Error ? cause.message : String(cause)))
      .finally(() => setLoading(false));
  }, []);

  const icons = useAppIcons(useMemo(() => sources.flatMap((source) => source.bundleIds), [sources]));
  const iconFor = (source: FollowUpSource) => source.bundleIds.map((id) => icons[id]).find(Boolean);

  const toggle = useCallback(async (source: FollowUpSource, enabled: boolean) => {
    const before = disabled;
    setDisabled(enabled ? disabled.filter((id) => id !== source.id) : [...disabled, source.id]);
    if (!nativeRuntime) return;
    try { setDisabled(await api.setFollowUpSourceEnabled(source.id, enabled)); setError(null); }
    catch (cause) { setDisabled(before); setError(cause instanceof Error ? cause.message : String(cause)); }
  }, [disabled]);

  // Soft fade at the scroll edges, shown only while there is more to see.
  const list = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState({ top: false, bottom: false });
  const measure = useCallback(() => {
    const el = list.current;
    if (el) setEdges({ top: el.scrollTop > 0, bottom: el.scrollTop < el.scrollHeight - el.clientHeight - 1 });
  }, []);
  useEffect(measure, [measure, sources]);

  const enabledCount = sources.filter((source) => !disabled.includes(source.id)).length;

  return <section aria-labelledby="follow-up-sources-title" className="settings-section">
    <div className="mb-3 flex items-start justify-between gap-4">
      <div><h2 id="follow-up-sources-title" className="text-sm font-semibold">Find follow-ups from</h2><p className="mt-1 text-xs leading-5 text-muted-foreground">Memento only looks for follow-ups in the apps you leave on.</p></div>
      <span className="shrink-0 pt-0.5 text-xs tabular-nums text-muted-foreground" aria-live="polite">{loading ? "" : `${enabledCount} of ${sources.length} on`}</span>
    </div>
    <div className="relative rounded-xl border bg-muted/35 shadow-sm">
      <div ref={list} onScroll={measure} className="max-h-72 overflow-y-auto" tabIndex={-1}>
        {loading ? <p className="px-4 py-5 text-center text-xs text-muted-foreground">Loading apps…</p> : CATEGORIES.map((category) => {
          const group = sources.filter((source) => source.category === category.id);
          if (group.length === 0) return null;
          return <div key={category.id} role="group" aria-labelledby={`fus-${category.id}`}>
            <h3 id={`fus-${category.id}`} className="sticky top-0 z-10 bg-muted px-4 py-1.5 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">{category.label}</h3>
            <ul className="divide-y">
              {group.map((source) => {
                const on = !disabled.includes(source.id);
                return <li key={source.id} className="flex min-h-11 items-center gap-3 px-4 transition-colors hover:bg-muted/60">
                  <AppIcon src={iconFor(source)} fallback={source.category === "meeting" ? Mic2Icon : source.category === "email" ? MailIcon : GlobeIcon} />
                  <span className="min-w-0 flex-1 truncate text-sm">{source.name}</span>
                  <span className="w-8 text-right text-xs text-muted-foreground">{on ? "On" : "Off"}</span>
                  <Switch checked={on} onCheckedChange={(next) => void toggle(source, next)} aria-label={`Find follow-ups from ${source.name}`} />
                </li>;
              })}
            </ul>
          </div>;
        })}
      </div>
      {edges.top ? <div aria-hidden="true" className="pointer-events-none absolute inset-x-0 top-0 h-8 rounded-t-xl bg-gradient-to-b from-background/80 to-transparent" /> : null}
      {edges.bottom ? <div aria-hidden="true" className="pointer-events-none absolute inset-x-0 bottom-0 h-8 rounded-b-xl bg-gradient-to-t from-background/80 to-transparent" /> : null}
    </div>
    {error ? <p role="alert" className="mt-2 text-xs text-destructive">{error}</p> : null}
  </section>;
}
