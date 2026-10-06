import { useEffect, useState } from "react";
import { ActionsView } from "@/components/actions-view";
import { MeetingsView } from "@/components/meetings-view";
import { NewMemoryDialog } from "@/components/new-memory-dialog";
import { Onboarding } from "@/components/onboarding";
import { SettingsView } from "@/components/settings-view";
import { Sidebar, type View } from "@/components/sidebar";
import { TimelineView } from "@/components/timeline-view";
import { TodayView } from "@/components/today-view";
import { api } from "@/lib/api";
import { BROWSE_EVENT } from "@/lib/lifecycle";
import { MementoProvider, useMemento } from "@/lib/store";

function Shell() {
  const { status, actions, newMemorySignal, setPaused, settings } = useMemento();
  const [view, setView] = useState<View>("today");
  const [addOpen, setAddOpen] = useState(false);
  const [memoryToOpen, setMemoryToOpen] = useState<number | null>(null);
  const [onboardingDismissed, setOnboardingDismissed] = useState(false);
  const [deepLinkQuery, setDeepLinkQuery] = useState("");

  useEffect(() => {
    if (newMemorySignal > 0) setAddOpen(true);
  }, [newMemorySignal]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (!event.metaKey || event.altKey || event.ctrlKey) return;
      const routes: Record<string, View> = {
        "1": "today",
        "2": "memory",
        "3": "actions",
        "4": "meetings",
      };
      if (routes[event.key]) {
        event.preventDefault();
        setView(routes[event.key]);
      } else if (event.key === ",") {
        event.preventDefault();
        setView("settings");
      } else if (event.key.toLowerCase() === "n") {
        event.preventDefault();
        setAddOpen(true);
      } else if (event.key.toLowerCase() === "f") {
        event.preventDefault();
        setView("memory");
        window.setTimeout(() => document.querySelector<HTMLInputElement>("#memory-search")?.focus(), 0);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let unlisten: (() => void) | undefined;
    import("@tauri-apps/api/event").then(({ listen }) => listen<string>("menu-command", ({ payload }) => {
      const routes: Partial<Record<string, View>> = { today: "today", memory: "memory", actions: "actions", meetings: "meetings", settings: "settings" };
      if (routes[payload]) setView(routes[payload]!);
      if (payload === "new-note") setAddOpen(true);
      if (payload === "find") { setView("memory"); window.setTimeout(() => document.querySelector<HTMLInputElement>("#memory-search")?.focus(), 0); }
    })).then((dispose) => { unlisten = dispose; });
    return () => unlisten?.();
  }, []);

  // memento://search?q=…  |  memento://today|memory|actions|meetings|settings|new
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let unlisten: (() => void) | undefined;
    import("@tauri-apps/api/event").then(({ listen }) => listen<string[]>("deep-link", ({ payload }) => {
      for (const raw of payload) {
        let url: URL;
        try { url = new URL(raw); } catch { continue; }
        if (url.protocol !== "memento:") continue;
        const target = (url.hostname || url.pathname.replace(/^\/+/, "")).toLowerCase();
        const views: Record<string, View> = { today: "today", memory: "memory", search: "memory", actions: "actions", meetings: "meetings", settings: "settings" };
        if (target === "new") setAddOpen(true);
        else if (views[target]) setView(views[target]);
        if (target === "search") setDeepLinkQuery(url.searchParams.get("q") ?? "");
        break;
      }
    })).then((dispose) => { unlisten = dispose; });
    return () => unlisten?.();
  }, []);

  // Data lifecycle: "View" on a source in Settings opens the Memory view (filtered there).
  useEffect(() => { const open = () => setView("memory"); window.addEventListener(BROWSE_EVENT, open); return () => window.removeEventListener(BROWSE_EVENT, open); }, []);

  async function saveNote(text: string) {
    await api.insertNote(text);
  }

  function handlePauseToggle() {
    setPaused(!(status?.paused ?? false));
  }

  return (
    <div className="flex h-full min-w-[760px] overflow-hidden bg-[var(--workspace)] text-foreground">
      <Sidebar
        view={view}
        onChange={setView}
        actionCount={actions.length}
        paused={status?.paused ?? false}
        onPauseToggle={handlePauseToggle}
        onRecord={() => setView("meetings")}
      />
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-[var(--workspace)]">
        <main className="flex min-h-0 flex-1 flex-col overflow-hidden">
          {view === "today" ? (
            <TodayView
              onAddNote={() => setAddOpen(true)}
              onOpenMemory={(id) => { setMemoryToOpen(id); setView("memory"); }}
              onOpenFollowUps={() => setView("actions")}
            />
          ) : null}
          {view === "memory" ? <TimelineView initialSelectedId={memoryToOpen} initialQuery={deepLinkQuery} /> : null}
          {view === "meetings" ? <MeetingsView /> : null}
          {view === "actions" ? <ActionsView /> : null}
          {view === "settings" ? <SettingsView /> : null}
        </main>
      </div>

      <NewMemoryDialog
        open={addOpen}
        onOpenChange={setAddOpen}
        onSave={saveNote}
      />

      {!settings.onboardingComplete && !onboardingDismissed ? (
        <Onboarding onDone={() => { setAddOpen(false); setOnboardingDismissed(true); }} />
      ) : null}
    </div>
  );
}

export default function App() {
  return (
    <MementoProvider>
      <Shell />
    </MementoProvider>
  );
}
