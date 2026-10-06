import { useCallback, useEffect, useState } from "react";
import { BrainCircuitIcon, CheckIcon, CloudIcon, DatabaseIcon, FolderOpenIcon, Mic2Icon, MonitorIcon, ServerIcon, ShieldCheckIcon, TerminalIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CaptureAppsSection } from "@/components/capture-apps-section";
import { CaptureHealthSection } from "@/components/capture-health-section";
import { DataLifecycleSection } from "@/components/data-lifecycle-section";
import { DesktopSection } from "@/components/desktop-section";
import { FollowUpSourcesSection } from "@/components/follow-up-sources-section";
import { McpSection } from "@/components/mcp-section";
import { ModelsSection } from "@/components/models-section";
import { VaultSection } from "@/components/vault-section";
import { Field, FieldContent, FieldDescription, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import { useMemento } from "@/lib/store";
import { cn } from "@/lib/utils";
import type { Permissions, PermState } from "@/lib/types";
import { TRANSCRIPTION_MODELS } from "@/lib/types";

type Pane = "capture" | "meetings" | "memory" | "advanced";
const panes = [
  { id: "capture" as const, label: "Capture & privacy", icon: ShieldCheckIcon },
  { id: "meetings" as const, label: "Meetings", icon: Mic2Icon },
  { id: "memory" as const, label: "Memory & vault", icon: BrainCircuitIcon },
  { id: "advanced" as const, label: "Advanced", icon: DatabaseIcon },
];

const enrichmentProviders = [
  { id: "ollama", label: "Ollama", detail: "Local server", icon: ServerIcon, baseUrl: "http://127.0.0.1:11434/v1", model: "llama3.2", fields: true, hosted: false },
  { id: "claude-code", label: "Claude Code", detail: "Local CLI · claude", icon: TerminalIcon, baseUrl: "", model: "claude-code", fields: false, hosted: false },
  { id: "codex-cli", label: "Codex", detail: "Local CLI · codex", icon: TerminalIcon, baseUrl: "", model: "codex-cli", fields: false, hosted: false },
  { id: "grok-cli", label: "Grok CLI", detail: "Local CLI · grok", icon: TerminalIcon, baseUrl: "", model: "grok-cli", fields: false, hosted: false },
  { id: "glm", label: "GLM", detail: "Coding Plan API", icon: CloudIcon, baseUrl: "https://api.z.ai/api/coding/paas/v4", model: "glm-4.7", fields: true, hosted: true },
  { id: "deepseek", label: "DeepSeek", detail: "Compatible API", icon: CloudIcon, baseUrl: "https://api.deepseek.com/v1", model: "deepseek-chat", fields: true, hosted: true },
] as const;

function SettingSection({ title, description, children }: { title: string; description?: string; children: React.ReactNode }) {
  return <section className="settings-section"><div className="mb-3"><h2 className="text-sm font-semibold">{title}</h2>{description ? <p className="mt-1 text-xs leading-5 text-muted-foreground">{description}</p> : null}</div><div className="settings-group">{children}</div></section>;
}

function PermissionRow({ title, description, state, onOpen }: { title: string; description: string; state: PermState | null; onOpen: () => Promise<void> }) {
  const granted = state === "granted";
  const pending = state === "notDetermined" || state === null;
  return <Field orientation="horizontal" className="settings-row"><FieldContent><FieldLabel>{title}</FieldLabel><FieldDescription>{description}</FieldDescription></FieldContent>{granted ? <Badge className="bg-success/10 text-success">Allowed</Badge> : <div className="flex items-center gap-2">{state === "restricted" ? <Badge className="bg-destructive/10 text-destructive">Restricted</Badge> : null}<Button variant="outline" size="sm" onClick={onOpen}>{pending ? "Allow" : "Open System Settings"}</Button></div>}</Field>;
}

function ToggleRow({ title, description, checked, onChange }: { title: string; description: string; checked: boolean; onChange: (value: boolean) => void }) {
  return <Field orientation="horizontal" className="settings-row"><FieldContent><FieldLabel>{title}</FieldLabel><FieldDescription>{description}</FieldDescription></FieldContent><Switch checked={checked} onCheckedChange={onChange} /></Field>;
}

export function SettingsView() {
  const { status, settings, saveSetting, setPaused, refreshStatus, refreshAll } = useMemento();
  const [pane, setPane] = useState<Pane>("capture");
  const [perms, setPerms] = useState<Permissions | null>(null);
  const [enrichmentStatus, setEnrichmentStatus] = useState<string | null>(null);
  const [enrichmentBusy, setEnrichmentBusy] = useState(false);
  const refreshPerms = useCallback(async () => setPerms(await api.getPermissions()), []);
  useEffect(() => { refreshPerms(); }, [refreshPerms]);
  useEffect(() => {
    if (pane !== "capture") return;
    const refreshOnFocus = () => refreshPerms();
    window.addEventListener("focus", refreshOnFocus);
    const timer = window.setInterval(refreshPerms, 1200);
    return () => { window.removeEventListener("focus", refreshOnFocus); window.clearInterval(timer); };
  }, [pane, refreshPerms]);
  async function requestAndRefresh(kind: string) {
    await api.requestPermission(kind);
    await refreshPerms();
  }
  const selectedProvider = enrichmentProviders.find((provider) => provider.id === settings.llmProvider) ?? enrichmentProviders[0];
  async function selectEnrichmentProvider(provider: (typeof enrichmentProviders)[number]) {
    setEnrichmentStatus(null);
    await saveSetting("llmProvider", provider.id);
    await saveSetting("llmModel", provider.model);
    if (provider.baseUrl) await saveSetting("llmBaseUrl", provider.baseUrl);
  }

  return <div className="workspace-page flex-row">
    <aside aria-label="Settings categories" className="settings-sidebar w-52 shrink-0 border-r bg-card/20 px-3 py-5">
      <p className="px-3 pb-3 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">Settings</p>
      <div className="flex flex-col gap-1">{panes.map(({ id, label, icon: Icon }) => <button key={id} className={cn("settings-nav", pane === id && "settings-nav-active")} aria-current={pane === id ? "page" : undefined} onClick={() => setPane(id)}><Icon />{label}</button>)}</div>
    </aside>
    <div className="min-w-0 flex-1 overflow-y-auto px-8 pb-12 lg:px-12"><div className="mx-auto max-w-3xl">
      <header className="workspace-header mb-7"><div><h1 className="workspace-title">{panes.find((item) => item.id === pane)?.label}</h1><p className="workspace-subtitle">Private by default. Your database and recordings stay on this Mac.</p></div></header>

      {pane === "capture" ? <div className="space-y-7">
        <SettingSection title="Automatic capture" description="Memento can quietly remember the apps you choose while you work.">
          <ToggleRow title="Remember my work" description="Pause anytime from the bottom of the sidebar." checked={!status?.paused} onChange={(v) => setPaused(!v)} /><Separator />
          <PermissionRow title="Accessibility" description="Reads the active app, window title, and visible text." state={perms?.accessibility ?? null} onOpen={() => requestAndRefresh("accessibility")} /><Separator />
          <PermissionRow title="Microphone" description="Captures your side of meetings and voice notes." state={perms?.microphone ?? null} onOpen={() => requestAndRefresh("microphone")} /><Separator />
          <PermissionRow title="Screen & system audio" description="Captures other participants during meetings. Reopen Memento after allowing it." state={perms?.screenRecording ?? null} onOpen={() => requestAndRefresh("screen-recording")} />
          <div className="flex justify-end border-t px-4 py-2"><Button variant="ghost" size="sm" onClick={refreshPerms}>Check permissions again</Button></div>
        </SettingSection><CaptureAppsSection /><FollowUpSourcesSection /><CaptureHealthSection /><DataLifecycleSection />{/* MOUNT-CAPTURE: add your capture-pane sections directly above this marker */}
      </div> : null}

      {pane === "meetings" ? <div className="space-y-7">
        <SettingSection title="Recording sources" description="Choose what Memento records when a meeting starts.">
          <ToggleRow title="Record microphone" description="Capture your voice during meetings." checked={settings.recordMic} onChange={(v) => saveSetting("recordMic", String(v))} /><Separator />
          <ToggleRow title="Record system audio" description="Capture the other people in the call." checked={settings.recordSystem} onChange={(v) => saveSetting("recordSystem", String(v))} /><Separator />
          <ToggleRow title="Mix audio before transcribing" description="Create one readable local transcript from both sources." checked={settings.mixAudio} onChange={(v) => saveSetting("mixAudio", String(v))} />
        </SettingSection>
        <SettingSection title="Automation"><ToggleRow title="Start when a meeting begins" description="Recognizes Meet, Zoom, Teams, and other meeting apps." checked={settings.autoStartMeetings} onChange={(v) => saveSetting("autoStartMeetings", String(v))} /><Separator /><ToggleRow title="Stop when the meeting ends" description="Stops shortly after you leave the meeting window." checked={settings.autoStopMeetings} onChange={(v) => saveSetting("autoStopMeetings", String(v))} /></SettingSection>
      </div> : null}

      {pane === "memory" ? <div className="space-y-7"><VaultSection />
        <SettingSection title="Smart summaries" description="Optionally turn raw activity into summaries, people, decisions, and follow-ups.">
          <ToggleRow title="Enrich captured memories" description="Create concise summaries, decisions, and follow-ups with an agent you already use." checked={settings.llmEnabled} onChange={(v) => saveSetting("llmEnabled", v)} />
          {settings.llmEnabled ? <div className="space-y-5 border-t p-4">
            <Field>
              <FieldLabel>Provider</FieldLabel>
              <FieldDescription>Local CLIs use your existing sign-in. Hosted APIs receive the captured text being enriched.</FieldDescription>
              <div role="radiogroup" aria-label="Memory enrichment provider" className="mt-2 grid grid-cols-2 gap-2 sm:grid-cols-3">
                {enrichmentProviders.map((provider) => { const selected = provider.id === selectedProvider.id; const Icon = provider.icon; return <button key={provider.id} type="button" role="radio" aria-checked={selected} onClick={() => selectEnrichmentProvider(provider)} className={cn("group relative min-w-0 rounded-lg border px-3 py-3 text-left transition-colors", selected ? "border-primary/50 bg-primary/10" : "border-border bg-card/40 hover:border-foreground/20 hover:bg-muted/50")}>
                  <div className="flex items-center gap-2"><Icon className={cn("size-4 shrink-0", selected ? "text-primary" : "text-muted-foreground")} /><span className="truncate text-xs font-medium">{provider.label}</span>{selected ? <CheckIcon className="ml-auto size-3.5 text-primary" /> : null}</div>
                  <p className="mt-1.5 truncate text-[11px] text-muted-foreground">{provider.detail}</p>
                </button>; })}
              </div>
            </Field>
            {!selectedProvider.fields ? <div className="rounded-lg border border-border/70 bg-muted/30 px-3 py-2.5"><div className="flex items-center gap-2 text-xs font-medium"><TerminalIcon className="size-3.5 text-muted-foreground" /> Uses the installed <code className="rounded bg-muted px-1 py-0.5">{selectedProvider.id === "claude-code" ? "claude" : selectedProvider.id === "codex-cli" ? "codex" : "grok"}</code> command</div><p className="mt-1 text-[11px] leading-4 text-muted-foreground">Memento runs one non-interactive turn in a temporary folder. Install and sign in to the CLI first.</p></div> : <div className="grid gap-4 sm:grid-cols-2">
              <Field className="sm:col-span-2"><FieldLabel htmlFor="llm-url">Base URL</FieldLabel><Input id="llm-url" value={settings.llmBaseUrl} onChange={(e) => saveSetting("llmBaseUrl", e.target.value)} /></Field>
              <Field><FieldLabel htmlFor="llm-model">Model</FieldLabel><Input id="llm-model" value={settings.llmModel} onChange={(e) => saveSetting("llmModel", e.target.value)} /></Field>
              <Field><FieldLabel htmlFor="llm-key">API key</FieldLabel><Input id="llm-key" type="password" placeholder={settings.llmHasApiKey ? "Saved — enter to replace" : selectedProvider.hosted ? "Required" : "Usually not needed"} onBlur={(e) => { if (e.target.value) { saveSetting("llmApiKey", e.target.value); e.target.value = ""; } }} /></Field>
            </div>}
            <div className="flex items-center gap-3"><Button variant="outline" size="sm" disabled={enrichmentBusy} onClick={async () => { setEnrichmentBusy(true); setEnrichmentStatus(null); try { const count = await api.enrichMemoriesNow(); await refreshAll(); setEnrichmentStatus(count ? `Enriched ${count} memories.` : "Everything is up to date."); } catch (error) { setEnrichmentStatus(error instanceof Error ? error.message : String(error)); } finally { setEnrichmentBusy(false); } }}>{enrichmentBusy ? "Enriching…" : "Enrich memories now"}</Button>{enrichmentStatus ? <p role="status" className="text-xs text-muted-foreground">{enrichmentStatus}</p> : null}</div>
          </div> : null}
        </SettingSection>
      </div> : null}

      {pane === "advanced" ? <div className="space-y-7"><DesktopSection /><McpSection />
        <SettingSection title="Local models" description="Your memory is an Obsidian-compatible Markdown vault — readable, editable, and portable. Only the transcription model needs downloading."><ModelsSection /><Separator /><div className="space-y-4 p-4"><Field><FieldLabel>Transcription model</FieldLabel><FieldDescription>Runs locally via whisper.cpp with Whisper Tiny.</FieldDescription><div role="group" aria-label="Transcription model" className="mt-1 flex w-fit gap-1 rounded-lg bg-muted/60 p-1">{TRANSCRIPTION_MODELS.map(({ id, label }) => { const selected = settings.transcriptionModel === id; return <button key={id} type="button" aria-pressed={selected} onClick={() => saveSetting("transcriptionModel", id)} className={cn("inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors", selected ? "border border-border bg-card text-foreground" : "text-muted-foreground hover:text-foreground")}>{label}</button>; })}</div></Field></div></SettingSection>
        <SettingSection title="Local database"><div className="p-4"><div className="flex items-center gap-2 text-sm font-medium"><FolderOpenIcon className="size-4" /> Memento data</div><p className="mt-2 break-all font-mono text-xs text-muted-foreground">{status?.dbPath ?? "…"}</p><p className="mt-2 text-xs text-muted-foreground">{status?.memoryCount ?? 0} memories · {status?.actionCount ?? 0} open follow-ups</p><Button variant="outline" size="sm" className="mt-3" onClick={refreshStatus}><MonitorIcon /> Refresh status</Button></div></SettingSection>
      </div> : null}
    </div></div>
  </div>;
}
