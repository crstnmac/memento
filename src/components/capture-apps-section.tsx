import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import { RefreshCwIcon, SearchIcon, XIcon } from "lucide-react";
import { AppIcon, useAppIcons } from "@/components/app-icon";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import { useMemento } from "@/lib/store";
import type { CaptureAppSelection } from "@/lib/types";

const nativeRuntime = "__TAURI_INTERNALS__" in window;
const REFRESH_COOLDOWN_MS = 3000;

function sameBundle(a: string, b: string) { return a.toLowerCase() === b.toLowerCase(); }

/** "https://www.Example.com/path" → "www.example.com"; same as the backend. */
function toHost(value: string): string {
  let host = value.trim().toLowerCase();
  if (host.includes("://")) {
    try { host = new URL(host).hostname; } catch { /* keep the raw text */ }
  }
  return host.split(/[/?#]/)[0].replace(/^\.+|\.+$/g, "");
}

const PREVIEW_APPS: CaptureAppSelection[] = [
  { name: "Google Chrome", bundleId: "com.google.Chrome" },
  { name: "Mail", bundleId: "com.apple.mail" },
  { name: "Memento", bundleId: "com.memento.app" },
  { name: "Notes", bundleId: "com.apple.Notes" },
  { name: "Slack", bundleId: "com.tinyspeck.slackmacgap" },
  { name: "WhatsApp", bundleId: "net.whatsapp.WhatsApp" },
];

/**
 * Denylist for ambient capture: every installed app can be switched off, and
 * whole websites can be blocked. Changes apply immediately and are saved as
 * they are made.
 */
export function CaptureAppsSection() {
  const { status } = useMemento();
  // Without Accessibility nothing is captured, so there is nothing to exclude.
  const capturing = !nativeRuntime || status?.trusted === true;
  const [apps, setApps] = useState<CaptureAppSelection[]>([]);
  const [excluded, setExcluded] = useState<CaptureAppSelection[]>([]);
  const [domains, setDomains] = useState<string[]>([]);
  const [domain, setDomain] = useState("");
  const [search, setSearch] = useState("");
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setBusy(true);
    if (!nativeRuntime) {
      setApps(PREVIEW_APPS);
      setBusy(false);
      return;
    }
    try {
      const [catalog, savedApps, savedDomains] = await Promise.all([api.listInstalledApps(), api.getExcludedApps(), api.getBlockedDomains()]);
      setApps(catalog); setExcluded(savedApps); setDomains(savedDomains); setError(null);
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(false); }
  }, []);

  useEffect(() => { void load(); }, [load]);

  // Apps get installed and removed while Settings sits open: refresh whenever
  // the window comes back to the front (throttled).
  useEffect(() => {
    let last = Date.now();
    const onFocus = () => {
      if (Date.now() - last < REFRESH_COOLDOWN_MS) return;
      last = Date.now();
      void load();
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [load]);

  const visibleApps = useMemo(() => {
    const merged = [...apps];
    // An excluded app that was uninstalled must stay listed so it can be re-enabled.
    for (const app of excluded) if (!merged.some((item) => sameBundle(item.bundleId, app.bundleId))) merged.push(app);
    return merged.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
  }, [apps, excluded]);
  const filteredApps = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return needle ? visibleApps.filter((app) => app.name.toLowerCase().includes(needle) || app.bundleId.toLowerCase().includes(needle)) : visibleApps;
  }, [visibleApps, search]);
  const icons = useAppIcons(useMemo(() => visibleApps.map((app) => app.bundleId), [visibleApps]));
  const isExcluded = (app: CaptureAppSelection) => excluded.some((item) => sameBundle(item.bundleId, app.bundleId));

  async function toggle(app: CaptureAppSelection) {
    const before = excluded;
    const next = isExcluded(app) ? excluded.filter((item) => !sameBundle(item.bundleId, app.bundleId)) : [...excluded, app];
    setExcluded(next);
    if (!nativeRuntime) return;
    try { setExcluded(await api.setExcludedApps(next)); setError(null); }
    catch (cause) { setExcluded(before); setError(cause instanceof Error ? cause.message : String(cause)); }
  }

  async function addDomain(event: FormEvent) {
    event.preventDefault();
    const host = toHost(domain);
    if (!host) return;
    if (!host.includes(".") || /\s/.test(host)) { setError("Enter a domain such as example.com."); return; }
    if (domains.includes(host)) { setDomain(""); setError(null); return; }
    try {
      const next = nativeRuntime ? await api.setBlockedDomains([...domains, host]) : [...domains, host];
      setDomains(next); setDomain(""); setError(null);
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  }

  async function removeDomain(value: string) {
    const next = domains.filter((item) => item !== value);
    try { setDomains(nativeRuntime ? await api.setBlockedDomains(next) : next); setError(null); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  }

  return <section aria-labelledby="data-management-title" className="settings-section">
    <div className="mb-3 flex items-start justify-between gap-4">
      <div><h2 id="data-management-title" className="text-sm font-semibold">Data management</h2><p className="mt-1 text-xs leading-5 text-muted-foreground">Exclude apps and domains from ambient capture.</p></div>
      {capturing ? <Button variant="ghost" size="sm" onClick={() => void load()} disabled={busy} aria-label="Refresh app list"><RefreshCwIcon className={busy ? "animate-spin" : ""} /> Refresh</Button> : null}
    </div>
    {!capturing ? <p className="rounded-xl border bg-muted/35 p-4 text-xs leading-5 text-muted-foreground">Allow Accessibility above to choose which apps and websites Memento leaves out.</p> : <div className="rounded-xl border bg-muted/35 p-4 shadow-sm">
      <div className="flex items-baseline justify-between gap-3">
        <div><h3 className="text-sm font-medium">Excluded apps</h3><p className="mt-1 text-xs text-muted-foreground">Turn on apps you don’t want Memento to capture.</p></div>
        <span className="shrink-0 text-xs tabular-nums text-muted-foreground" aria-live="polite">{excluded.length === 0 ? "None excluded" : `${excluded.length} excluded`}</span>
      </div>
      <div className="relative mt-3">
        <SearchIcon aria-hidden="true" className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Search apps" aria-label="Search apps" className="pl-8" />
      </div>
      <div className="mt-2 max-h-64 overflow-y-auto rounded-lg border bg-background">
        {filteredApps.length === 0 ? <p className="px-4 py-5 text-center text-xs text-muted-foreground">{busy ? "Loading apps…" : visibleApps.length === 0 ? "No apps detected" : "No match"}</p> : <ul className="divide-y">
          {filteredApps.map((app) => {
            const off = isExcluded(app);
            return <li key={app.bundleId} className="flex min-h-11 items-center gap-3 px-3 transition-colors hover:bg-muted/50">
              <AppIcon src={icons[app.bundleId]} />
              <span className="min-w-0 flex-1 truncate text-sm">{app.name}</span>
              <span className="w-16 text-right text-xs text-muted-foreground">{off ? "Excluded" : "Capturing"}</span>
              <Switch checked={off} onCheckedChange={() => void toggle(app)} aria-label={`Exclude ${app.name} from capture`} />
            </li>;
          })}
        </ul>}
      </div>
      <div className="mt-5"><h3 className="text-sm font-medium">Blocked domains</h3><p className="mt-1 text-xs text-muted-foreground">Never capture content from these hosts, including their subdomains.</p>
        <form onSubmit={addDomain} className="mt-3 flex gap-2"><Input value={domain} onChange={(event) => setDomain(event.target.value)} placeholder="e.g. example.com" aria-label="Domain to block" autoCapitalize="off" autoCorrect="off" spellCheck={false} /><Button type="submit" disabled={!domain.trim()}>Add</Button></form>
        {domains.length > 0 ? <ul className="mt-3 flex flex-wrap gap-1.5" aria-label="Blocked domains">{domains.map((item) => <li key={item} className="inline-flex items-center gap-1 rounded-full border bg-background py-1 pl-3 pr-1.5 text-xs"><span>{item}</span><button type="button" onClick={() => void removeDomain(item)} className="rounded-full p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-label={`Unblock ${item}`}><XIcon className="size-3.5" /></button></li>)}</ul> : null}
      </div>
    </div>}
    {error ? <p role="alert" className="mt-2 text-xs text-destructive">{error}</p> : null}
  </section>;
}
