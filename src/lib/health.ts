import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type HealthState =
  | "capturing"
  | "paused"
  | "needs-permission"
  | "idle"
  | "not-supported"
  | "unreadable"
  | "excluded";

export interface HealthDecision {
  reason: string;
  label: string;
  explanation: string;
  hint: string | null;
}

export interface RecentDecision extends HealthDecision {
  appName: string;
  bundleId: string | null;
  at: number;
  count: number;
  tunable: boolean;
}

export interface CaptureHealth {
  state: HealthState;
  headline: string;
  trusted: boolean;
  paused: boolean;
  frontmost: { name: string; bundleId: string | null } | null;
  lastDecision: HealthDecision | null;
  lastDecisionAt: number | null;
  lastCaptureAt: number | null;
  consecutiveUnreadable: number;
  recent: RecentDecision[];
}

export const healthApi = {
  get: () => invoke<CaptureHealth>("get_capture_health"),
  intervals: () => invoke<Record<string, number>>("get_app_capture_intervals"),
  setInterval: (bundleId: string, ms: number | null) =>
    invoke<Record<string, number>>("set_app_capture_interval", { bundleId, ms }),
  onChange: (handler: (health: CaptureHealth) => void): (() => void) => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen<CaptureHealth>("capture-health", (event) => handler(event.payload)).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  },
};

/** Per-app capture speed choices; `null` follows the global setting. */
export const SPEED_CHOICES: { value: number | null; label: string }[] = [
  { value: null, label: "Default" },
  { value: 5000, label: "Faster (5s)" },
  { value: 30000, label: "Normal (30s)" },
  { value: 120000, label: "Slower (2 min)" },
];
