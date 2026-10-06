import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { api, onEvent } from "./api";
import type {
  ActionItem,
  AppVisit,
  Conversation,
  DueReminders,
  MemoryRow,
  Settings,
  Status,
} from "./types";
import { DEFAULT_TRANSCRIPTION_MODEL } from "./types";

interface StoreValue {
  memories: MemoryRow[];
  conversations: Conversation[];
  actions: ActionItem[];
  visits: AppVisit[];
  status: Status | null;
  settings: Settings;
  transcribing: Set<number>;
  transcriptionErrors: Record<number, string>;
  dueReminders: DueReminders | null;
  newMemorySignal: number;
  versionsSignal: number;
  refreshAll: () => Promise<void>;
  refreshMemories: () => Promise<void>;
  refreshConversations: () => Promise<void>;
  refreshActions: () => Promise<void>;
  refreshVisits: () => Promise<void>;
  refreshStatus: () => Promise<void>;
  setPaused: (paused: boolean) => Promise<void>;
  saveSetting: (key: string, value: string | boolean | number) => Promise<void>;
  markActionsRead: () => Promise<void>;
  dismissReminders: (ids: number[]) => Promise<void>;
}

const MEMORY_WINDOW = 500;
const StoreContext = createContext<StoreValue | null>(null);
const nativeRuntime = "__TAURI_INTERNALS__" in window;

function defaults(): Settings {
  return {
    transcriptionModel: DEFAULT_TRANSCRIPTION_MODEL,
    recordMic: true,
    recordSystem: true,
    mixAudio: true,
    captureIntervalMs: 30000,
    onboardingComplete: !nativeRuntime,
    autoStartMeetings: false,
    autoStopMeetings: true,
    maxCaptureDurationSec: 0,
    reminderNudges: true,
    mcpEnabled: false,
    mcpPort: 3961,
    llmEnabled: false,
    llmProvider: "ollama",
    llmBaseUrl: "http://127.0.0.1:11434/v1",
    llmModel: "llama3.2",
    llmHasApiKey: false,
    vaultPath: "",
    recordingKind: "meeting",
  };
}

export function MementoProvider({ children }: { children: ReactNode }) {
  const [memories, setMemories] = useState<MemoryRow[]>([]);
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [actions, setActions] = useState<ActionItem[]>([]);
  const [visits, setVisits] = useState<AppVisit[]>([]);
  const [status, setStatus] = useState<Status | null>(null);
  const [settings, setSettings] = useState<Settings>(defaults);
  const [transcribing, setTranscribing] = useState<Set<number>>(new Set());
  const [transcriptionErrors, setTranscriptionErrors] = useState<
    Record<number, string>
  >({});
  const [newMemorySignal, setNewMemorySignal] = useState(0);
  const [versionsSignal, setVersionsSignal] = useState(0);
  const [dueReminders, setDueReminders] = useState<DueReminders | null>(null);

  const settingsRef = useRef(settings);
  settingsRef.current = settings;

  const refreshMemories = useCallback(async () => {
    try {
      setMemories(await api.listMemories(MEMORY_WINDOW));
    } catch (e) {
      console.error(e);
    }
  }, []);

  const refreshConversations = useCallback(async () => {
    try {
      const rows = await api.listConversations(100);
      setConversations(rows);
      // Drop error state for recordings that no longer exist.
      setTranscriptionErrors((prev) => {
        const alive = new Set(rows.map((c) => c.id));
        const next: Record<number, string> = {};
        for (const [id, message] of Object.entries(prev)) {
          if (alive.has(Number(id))) next[Number(id)] = message;
        }
        return next;
      });
    } catch (e) {
      console.error(e);
    }
  }, []);

  const refreshActions = useCallback(async () => {
    try {
      setActions(await api.listActionItems("open"));
    } catch (e) {
      console.error(e);
    }
  }, []);

  const refreshVisits = useCallback(async () => {
    try {
      setVisits(await api.listVisits(100));
    } catch (e) {
      console.error(e);
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await api.status());
    } catch (e) {
      console.error(e);
    }
  }, []);

  const refreshAll = useCallback(async () => {
    await Promise.all([
      refreshMemories(),
      refreshConversations(),
      refreshActions(),
      refreshVisits(),
      refreshStatus(),
    ]);
  }, [refreshMemories, refreshConversations, refreshActions, refreshVisits, refreshStatus]);

  // Initial load + event subscription.
  useEffect(() => {
    if (!nativeRuntime) return;
    refreshAll();
  }, [refreshAll]);

  useEffect(() => {
    if (!nativeRuntime) return;
    const unsub = onEvent((event) => {
      if (event.event === "memory-new") {
        const memory = event.payload as MemoryRow;
        // Keep the in-memory list bounded to what refreshMemories loads.
        setMemories((prev) => [
          memory,
          ...prev.filter((m) => m.id !== memory.id),
        ].slice(0, MEMORY_WINDOW));
      } else if (event.event === "pause-changed") {
        refreshStatus();
      } else if (event.event === "new-memory") {
        setNewMemorySignal((n) => n + 1);
      } else if (event.event === "recording-stopped") {
        // The backend saves the WAVs and starts transcription on its own; the
        // `transcription-*` events drive the UI from here.
        refreshConversations();
      } else if (event.event === "recording-started") {
        refreshStatus();
      } else if (event.event === "recording-auto-stopped") {
        refreshAll();
      } else if (event.event === "transcription-started") {
        const { conversationId } = event.payload;
        setTranscribing((prev) => new Set(prev).add(conversationId));
        setTranscriptionErrors((prev) => {
          if (!(conversationId in prev)) return prev;
          const next = { ...prev };
          delete next[conversationId];
          return next;
        });
      } else if (event.event === "transcription-finished") {
        const { conversationId } = event.payload;
        setTranscribing((prev) => {
          const next = new Set(prev);
          next.delete(conversationId);
          return next;
        });
        refreshConversations();
      } else if (event.event === "transcription-failed") {
        const { conversationId, error } = event.payload;
        setTranscribing((prev) => {
          const next = new Set(prev);
          next.delete(conversationId);
          return next;
        });
        setTranscriptionErrors((prev) => ({ ...prev, [conversationId]: error }));
        refreshConversations();
      } else if (event.event === "reminders-due") {
        setDueReminders(event.payload as DueReminders);
        if (settingsRef.current.reminderNudges) refreshActions();
      } else if (event.event === "versions-updated") {
        setVersionsSignal((n) => n + 1);
      } else if (event.event === "agent-updated") {
        refreshActions();
      } else if (event.event === "memories-enriched") {
        refreshMemories();
        refreshActions();
      } else if (event.event === "visit") {
        refreshStatus();
      }
    });
    return unsub;
  }, [refreshAll, refreshConversations, refreshStatus, refreshActions]);

  const markActionsRead = useCallback(async () => {
    try {
      const open = await api.listActionItems("open");
      const unread = open.filter((a) => a.isUnread);
      for (const item of unread) {
        await api.updateActionItem(item.id, { unread: false });
      }
      if (unread.length > 0) setActions((prev) =>
        prev.map((a) => ({ ...a, isUnread: false })),
      );
    } catch (e) {
      console.error(e);
    }
  }, []);

  const dismissReminders = useCallback(async (ids: number[]) => {
    setDueReminders(null);
    if (ids.length === 0) {
      const open = await api.listActionItems("open");
      const timed = open.filter((a) => a.remindAt !== null);
      ids = timed.map((a) => a.id);
    }
    await api.markRemindersShown(ids).catch(console.error);
  }, []);

  useEffect(() => {
    if (!nativeRuntime) return;
    // Merge with defaults so frontend-only settings (recordingKind, etc.)
    // survive a load where the backend struct doesn't include them.
    api
      .settings()
      .then((s) => setSettings((prev) => ({ ...prev, ...s })))
      .catch(console.error);
  }, []);

  const value = useMemo<StoreValue>(
    () => ({
      memories,
      conversations,
      actions,
      visits,
      status,
      settings,
      transcribing,
      transcriptionErrors,
      dueReminders,
      newMemorySignal,
      versionsSignal,
      refreshAll,
      refreshMemories,
      refreshConversations,
      refreshActions,
      refreshVisits,
      refreshStatus,
      setPaused: async (paused) => {
        await api.setPaused(paused);
        await refreshStatus();
      },
      saveSetting: async (key, value) => {
        setSettings((prev) => {
          const current = prev[key as keyof Settings];
          const next = typeof current === "boolean"
            ? String(value) === "true"
            : typeof current === "number"
              ? Number(value)
              : value;
          return { ...prev, [key]: next } as Settings;
        });
        if (nativeRuntime) await api.setSetting(key, String(value));
      },
      markActionsRead,
      dismissReminders,
    }),
    [
      memories,
      conversations,
      actions,
      visits,
      status,
      settings,
      transcribing,
      transcriptionErrors,
      dueReminders,
      newMemorySignal,
      versionsSignal,
      refreshAll,
      refreshMemories,
      refreshConversations,
      refreshActions,
      refreshVisits,
      refreshStatus,
      markActionsRead,
      dismissReminders,
    ],
  );

  return <StoreContext.Provider value={value}>{children}</StoreContext.Provider>;
}

export function useMemento(): StoreValue {
  const ctx = useContext(StoreContext);
  if (!ctx) throw new Error("useMemento must be used within MementoProvider");
  return ctx;
}
