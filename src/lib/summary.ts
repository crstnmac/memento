import { invoke } from "@tauri-apps/api/core";

export interface PeriodSummary {
  period: "day" | "week";
  /** Inclusive local dates, YYYY-MM-DD. */
  startDate: string;
  endDate: string;
  startMs: number;
  endMs: number;
  ongoing: boolean;
  memoryCount: number;
  firstAt: number | null;
  lastAt: number | null;
  apps: { app: string; count: number }[];
  topics: { title: string; app: string | null; count: number; memoryId: number; lastAt: number }[];
  meetings: {
    id: number;
    title: string;
    kind: string;
    startedAt: number;
    durationMs: number | null;
    memoryId: number | null;
  }[];
  followUps: {
    created: number;
    resolved: number;
    stillOpen: number;
    items: { id: number; content: string; status: string; memoryId: number | null }[];
  };
  busiestHour: { hour: number; count: number } | null;
  /** Per-day counts; weeks only. */
  days: { date: string; count: number }[];
  narrative: string | null;
}

export interface SnoozePreset {
  id: string;
  label: string;
  untilMs: number;
}

export const summaryApi = {
  /** `withNarrative: false` returns immediately (cached narrative only). */
  daySummary: (date: string, withNarrative = true) =>
    invoke<PeriodSummary>("get_day_summary", { date, withNarrative }),
  weekSummary: (weekStart: string, withNarrative = true) =>
    invoke<PeriodSummary>("get_week_summary", { weekStart, withNarrative }),
  editActionItemText: (id: number, text: string) =>
    invoke<string>("edit_action_item_text", { id, text }),
  snoozeActionItem: (id: number, untilMs: number) =>
    invoke<void>("snooze_action_item", { id, untilMs }),
  snoozePresets: () => invoke<SnoozePreset[]>("get_snooze_presets"),
};

export function toYmd(date: Date): string {
  const m = String(date.getMonth() + 1).padStart(2, "0");
  const d = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${m}-${d}`;
}

export function fromYmd(ymd: string): Date {
  const [y, m, d] = ymd.split("-").map(Number);
  return new Date(y, m - 1, d);
}

export function addDays(date: Date, days: number): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + days);
}

/** Monday of the week containing `date`. */
export function weekStartOf(date: Date): Date {
  return addDays(date, -((date.getDay() + 6) % 7));
}
