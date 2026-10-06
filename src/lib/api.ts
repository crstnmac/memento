import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  ActivityConversation,
  ActionItem,
  AppVisit,
  CaptureAppSelection,
  FollowUpSources,
  Conversation,
  ConversationEvent,
  DueReminders,
  McpState,
  MemoryRow,
  MemoryEnrichment,
  MemoryVersion,
  Permissions,
  RecordingAutoStopped,
  RecordingMode,
  RecordingResult,
  RecordingStart,
  Settings,
  Status,
  ThreadRow,
  TranscriptionFailed,
  TranscriptionFinished,
  TranscriptionStarted,
  TranscriptionModelStatus,
  ModelDownloadProgress,
  VaultStatus,
  VaultSyncReport,
} from "./types";

export const api = {
  status: () => invoke<Status>("get_status"),
  settings: () => invoke<Settings>("get_settings"),
  setSetting: (key: string, value: string) =>
    invoke<void>("set_setting", { key, value }),
  vaultStatus: () => invoke<VaultStatus>("vault_status"),
  vaultSyncNow: () => invoke<VaultSyncReport>("vault_sync_now"),
  vaultOpen: (inObsidian = false) =>
    invoke<void>("vault_open", { inObsidian }),
  getExcludedApps: () => invoke<CaptureAppSelection[]>("get_excluded_apps"),
  setExcludedApps: (apps: CaptureAppSelection[]) =>
    invoke<CaptureAppSelection[]>("set_excluded_apps", { apps }),
  getBlockedDomains: () => invoke<string[]>("get_blocked_domains"),
  setBlockedDomains: (domains: string[]) =>
    invoke<string[]>("set_blocked_domains", { domains }),
  listInstalledApps: () => invoke<CaptureAppSelection[]>("list_installed_apps"),
  /** Icons as data URLs, keyed by the bundle id that was requested. */
  getAppIcons: (bundleIds: string[]) =>
    invoke<Record<string, string>>("get_app_icons", { bundleIds }),
  getFollowUpSources: () => invoke<FollowUpSources>("get_follow_up_sources"),
  setFollowUpSourceEnabled: (id: string, enabled: boolean) =>
    invoke<string[]>("set_follow_up_source_enabled", { id, enabled }),
  getPermissions: () => invoke<Permissions>("get_permissions"),
  requestPermission: (kind: string) => invoke<void>("request_permission", { kind }),
  frontmost: () =>
    invoke<{
      pid: number | null;
      app: string | null;
      windowTitle: string | null;
    }>("get_frontmost"),
  focused: () =>
    invoke<{
      pid: number | null;
      app: string | null;
      windowTitle: string | null;
      focusedText: string | null;
      selectedText: string | null;
    }>("get_focused"),

  desktopActivate: (appName: string) =>
    invoke<boolean>("desktop_activate", { appName }),
  desktopFind: (pid: number, roles: string[], label: string) =>
    invoke<number | null>("desktop_find", { pid, roles, label }),
  desktopSetText: (handle: number, text: string) =>
    invoke<boolean>("desktop_set_text", { handle, text }),
  desktopPress: (handle: number) => invoke<boolean>("desktop_press", { handle }),
  desktopFocus: (handle: number) => invoke<boolean>("desktop_focus", { handle }),
  desktopGetBounds: (handle: number) =>
    invoke<[number, number, number, number] | null>("desktop_get_bounds", {
      handle,
    }),
  desktopMoveCursor: (
    x: number,
    y: number,
    opts?: {
      steps?: number;
      stepDelayMs?: number;
      preClickDelayMs?: number;
      holdMs?: number;
      doClick?: boolean;
    },
  ) =>
    invoke<void>("desktop_move_cursor", {
      x,
      y,
      steps: opts?.steps,
      stepDelayMs: opts?.stepDelayMs,
      preClickDelayMs: opts?.preClickDelayMs,
      holdMs: opts?.holdMs,
      doClick: opts?.doClick,
    }),

  listMemories: async (limit = 200, offset = 0) =>
    attachEnrichments(await invoke<MemoryRow[]>("list_memories", { limit, offset })),
  getMemory: async (id: number) => {
    const memory = await invoke<MemoryRow | null>("get_memory", { id });
    return memory ? (await attachEnrichments([memory]))[0] : null;
  },
  deleteMemory: (id: number) => invoke<void>("delete_memory", { id }),
  insertNote: (text: string) => invoke<MemoryRow>("insert_note", { text }),
  searchText: async (query: string) =>
    attachEnrichments(await invoke<MemoryRow[]>("search_memories", { query })),
  enrichMemoriesNow: () => invoke<number>("enrich_memories_now"),

  listVisits: (limit = 100) => invoke<AppVisit[]>("list_visits", { limit }),

  listThreads: (limit = 100) =>
    invoke<ThreadRow[]>("list_threads", { limit }),
  getThread: async (threadId: number) => {
    const thread = await invoke<[ThreadRow, MemoryRow[]] | null>("get_thread", { threadId });
    return thread ? [thread[0], await attachEnrichments(thread[1])] as [ThreadRow, MemoryRow[]] : null;
  },
  getThreadVersions: (threadId: number) =>
    invoke<MemoryVersion[]>("get_thread_versions", { threadId }),

  mcpCall: (tool: string, args: Record<string, unknown> = {}) =>
    invoke<unknown>("mcp_call", { tool, args }),
  mcpState: () => invoke<McpState>("mcp_state"),
  mcpSetEnabled: (enabled: boolean, port?: number) =>
    invoke<McpState>("mcp_set_enabled", { enabled, port }),
  mcpInstallClaudeDesktop: () =>
    invoke<string>("mcp_install_claude_desktop"),

  listActionItems: (status = "open") =>
    invoke<ActionItem[]>("list_action_items", { status }),
  setActionItemStatus: (id: number, status: string) =>
    invoke<void>("set_action_item_status", { id, status }),
  updateActionItem: (id: number, patch: {
    urgent?: boolean;
    remindAt?: number | null;
    unread?: boolean;
    sortOrder?: number;
  }) =>
    invoke<void>("update_action_item", { id, ...patch }),

  getDueReminders: () => invoke<DueReminders>("get_due_reminders"),
  getDndState: () => invoke<boolean>("get_dnd_state"),
  markRemindersShown: (ids: number[]) =>
    invoke<void>("mark_reminders_shown", { ids }),

  listConversations: (limit = 50) =>
    invoke<Conversation[]>("list_conversations", { limit }),
  getConversation: (id: number) =>
    invoke<Conversation | null>("get_conversation", { id }),
  listActivityConversations: (limit = 100) =>
    invoke<ActivityConversation[]>("list_activity_conversations", { limit }),
  getActivityConversationEvents: (conversationId: number, limit = 200) =>
    invoke<ConversationEvent[]>("get_activity_conversation_events", {
      conversationId,
      limit,
    }),
  deleteConversation: (id: number) =>
    invoke<void>("delete_conversation", { id }),
  startRecording: (mode: RecordingMode, kind: "meeting" | "voice-note") =>
    invoke<RecordingStart>("start_recording", { mode, kind }),
  stopRecording: () => invoke<RecordingResult>("stop_recording"),
  recordingStatus: () =>
    invoke<RecordingStart | null>("recording_status"),
  transcribeConversation: (conversationId: number) =>
    invoke<void>("transcribe_conversation", { conversationId }),
  chatAsk: (question: string) => invoke<string>("chat_ask", { question }),
  overlayExpand: () => invoke<void>("overlay_expand"),
  overlayCollapse: () => invoke<void>("overlay_collapse"),
  overlayHide: () => invoke<void>("overlay_hide"),
  overlayView: () => invoke<"pill" | "chat" | "hidden">("overlay_view"),
  setPaused: (paused: boolean, durationMin?: number) =>
    invoke<void>("set_paused", { paused, durationMin: durationMin ?? null }),
  prepareTranscriptionModel: (model?: string) =>
    invoke<void>("prepare_transcription_model", { model: model ?? null }),
  transcriptionModelStatus: (model?: string) =>
    invoke<TranscriptionModelStatus>("transcription_model_status", {
      model: model ?? null,
    }),
  deleteTranscriptionModel: (model?: string) =>
    invoke<void>("delete_transcription_model", { model: model ?? null }),
  agentRun: (kind: string, prompt: string) =>
    invoke<{ id: number; result: string }>("agent_run", { kind, prompt }),
};

async function attachEnrichments(memories: MemoryRow[]): Promise<MemoryRow[]> {
  if (memories.length === 0) return memories;
  const rows = await invoke<MemoryEnrichment[]>("list_memory_enrichments", {
    ids: memories.map((memory) => memory.id),
  });
  const byId = new Map(rows.map((row) => [row.memoryId, row]));
  return memories.map((memory) => ({ ...memory, enrichment: byId.get(memory.id) }));
}

export type MementoEvent =
  | { event: "memory-new"; payload: MemoryRow }
  | { event: "recording-stopped"; payload: RecordingResult }
  | { event: "recording-started"; payload: RecordingStart }
  | { event: "recording-auto-stopped"; payload: RecordingAutoStopped }
  | { event: "transcription-started"; payload: TranscriptionStarted }
  | { event: "transcription-finished"; payload: TranscriptionFinished }
  | { event: "transcription-failed"; payload: TranscriptionFailed }
  | { event: "model-download-progress"; payload: ModelDownloadProgress }
  | { event: "reminders-due"; payload: DueReminders }
  | { event: "versions-updated"; payload: number }
  | { event: "agent-updated"; payload: unknown }
  | { event: "memories-enriched"; payload: unknown }
  | { event: "pause-changed"; payload: boolean }
  | { event: "new-memory"; payload: unknown }
  | { event: "visit"; payload: unknown };

export function onEvent(handler: (e: MementoEvent) => void): () => void {
  const unsubs: Array<() => void> = [];
  let disposed = false;
  const targets: MementoEvent["event"][] = [
    "memory-new",
    "recording-stopped",
    "recording-started",
    "recording-auto-stopped",
    "transcription-started",
    "transcription-finished",
    "transcription-failed",
    "model-download-progress",
    "reminders-due",
    "versions-updated",
    "agent-updated",
    "memories-enriched",
    "pause-changed",
    "new-memory",
    "visit",
  ];
  for (const event of targets) {
    listen<MementoEvent["payload"]>(event, (e) =>
      handler({ event, payload: e.payload } as MementoEvent),
    ).then((unlisten) => {
      if (disposed) unlisten();
      else unsubs.push(unlisten);
    });
  }
  return () => {
    disposed = true;
    for (const unsub of unsubs) unsub();
  };
}
