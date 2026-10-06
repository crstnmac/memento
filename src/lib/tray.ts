import { invoke } from "@tauri-apps/api/core";

import type { CaptureHealth } from "./health";
import type { ActionItem, Permissions, RecordingMode, Settings } from "./types";

export { timeAgo, formatTime } from "./format";

export type MainView = "today" | "memory" | "actions" | "meetings" | "settings";

export interface TrayStatus {
  trusted: boolean;
  paused: boolean;
  recording: boolean;
  memoryCount: number;
  actionCount: number;
  dbPath: string;
  /** Epoch ms when a timed pause ends; null/absent for "until I resume". */
  pauseUntil?: number | null;
}

export interface ActiveRecording {
  conversationId: number;
  startedMs: number;
}

export const trayApi = {
  health: () => invoke<CaptureHealth>("get_capture_health"),
  status: () => invoke<TrayStatus>("get_status"),
  settings: () => invoke<Settings>("get_settings"),
  permissions: () => invoke<Permissions>("get_permissions"),
  requestAccessibility: () => invoke<void>("request_permission", { kind: "accessibility" }),
  requestPermission: (kind: string) => invoke<void>("request_permission", { kind }),
  recordingStatus: () => invoke<ActiveRecording | null>("recording_status"),
  startRecording: (mode: RecordingMode, kind: "meeting" | "voice-note" = "meeting") =>
    invoke<ActiveRecording>("start_recording", { mode, kind }),
  stopRecording: () => invoke<unknown>("stop_recording"),
  setPaused: (paused: boolean, durationMin?: number) =>
    invoke<void>("set_paused", durationMin ? { paused, durationMin } : { paused }),
  openFollowUps: () => invoke<ActionItem[]>("list_action_items", { status: "open" }),
  resolveFollowUp: (id: number) =>
    invoke<void>("set_action_item_status", { id, status: "resolved" }),
  insertNote: (text: string) => invoke<unknown>("insert_note", { text }),
  hide: () => invoke<null>("tray_panel_hide"),
  openMainView: (view: MainView) => invoke<null>("open_main_view", { view }),
};

/** mm:ss (or h:mm:ss past an hour) for a running timer. */
export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => n.toString().padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

/** Due-first (overdue/soonest reminder), then newest. */
export function pickFollowUps(items: ActionItem[], now = Date.now(), limit = 3): ActionItem[] {
  const due = (a: ActionItem) => a.remindAt != null && a.remindAt <= now;
  return [...items]
    .sort((a, b) => {
      if (due(a) !== due(b)) return due(a) ? -1 : 1;
      if (due(a) && due(b)) return (a.remindAt ?? 0) - (b.remindAt ?? 0);
      return b.createdAt - a.createdAt;
    })
    .slice(0, limit);
}

/** "Due 3:40 PM", "Overdue since 9:10 AM", "Due tomorrow" or null. */
export function dueLabel(remindAt: number | null, now = Date.now()): string | null {
  if (remindAt == null) return null;
  const d = new Date(remindAt);
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const sameDay = d.toDateString() === new Date(now).toDateString();
  if (remindAt <= now) {
    return sameDay ? `Overdue, was due ${time}` : `Overdue, was due ${d.toLocaleDateString([], { month: "short", day: "numeric" })}`;
  }
  if (sameDay) return `Due ${time}`;
  const tomorrow = new Date(now + 86400000).toDateString() === d.toDateString();
  return tomorrow ? `Due tomorrow, ${time}` : `Due ${d.toLocaleDateString([], { month: "short", day: "numeric" })}, ${time}`;
}
