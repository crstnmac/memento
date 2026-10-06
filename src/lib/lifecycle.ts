import { invoke } from "@tauri-apps/api/core";

import type { MemoryEnrichment, MemoryRow } from "@/lib/types";

export interface CaptureSource {
  /** Source id, "notes", or "app:<name>" for captures without a source tag. */
  id: string;
  kind: "source" | "notes" | "app";
  name: string;
  category: string;
  /** Set when kind === "app". */
  app: string | null;
  memories: number;
  firstAt: number | null;
  lastAt: number | null;
  bytes: number;
}

export interface CaptureSummary {
  sources: CaptureSource[];
  totals: { memories: number; firstAt: number | null; lastAt: number | null; bytes: number; recordings: number };
  disk: { databaseBytes: number; recordingsBytes: number; vaultBytes: number };
}

export interface DeleteFilter {
  sourceIds?: string[];
  /** App names of captures without a source tag. */
  apps?: string[];
  /** Inclusive. */
  sinceMs?: number;
  /** Exclusive. */
  untilMs?: number;
  includeNotes?: boolean;
  includeRecordings?: boolean;
  /** Required to delete without any source or time restriction. */
  all?: boolean;
}

export interface DeleteSummary {
  memories: number;
  notes: number;
  threads: number;
  followUps: number;
  activityConversations: number;
  recordings: number;
  recordingFiles: number;
  vaultFiles: number;
  failedFiles: number;
  bytesFreed: { database: number; recordings: number; vault: number; total: number };
}

export interface BrowseTarget {
  sourceId?: string;
  app?: string;
}

/** Selects a source row's own data in a delete filter. */
export function filterForSource(source: CaptureSource): DeleteFilter {
  if (source.kind === "app") return { apps: [source.app ?? source.name] };
  return { sourceIds: [source.id], includeNotes: source.kind === "notes" };
}

export function browseTargetFor(source: CaptureSource): BrowseTarget {
  return source.kind === "app" ? { app: source.app ?? source.name } : { sourceId: source.id };
}

export const lifecycle = {
  getCaptureSummary: () => invoke<CaptureSummary>("get_capture_summary"),
  previewDelete: (filter: DeleteFilter) => invoke<DeleteSummary>("preview_delete", { filter }),
  deleteMemories: (filter: DeleteFilter) => invoke<DeleteSummary>("delete_memories", { filter }),
  listMemoriesFiltered: async (
    target: BrowseTarget,
    range: { sinceMs?: number; untilMs?: number } = {},
    limit = 200,
    offset = 0,
  ): Promise<MemoryRow[]> => {
    const rows = await invoke<MemoryRow[]>("list_memories_filtered", {
      sourceId: target.sourceId,
      app: target.app,
      sinceMs: range.sinceMs,
      untilMs: range.untilMs,
      limit,
      offset,
    });
    if (rows.length === 0) return rows;
    const enrichments = await invoke<MemoryEnrichment[]>("list_memory_enrichments", { ids: rows.map((row) => row.id) });
    const byId = new Map(enrichments.map((row) => [row.memoryId, row]));
    return rows.map((row) => ({ ...row, enrichment: byId.get(row.id) }));
  },
};

// --- Opening the Memory view filtered to one source -------------------------
// The settings section can't reach the Memory view directly, so it posts a
// window event; App.tsx switches views and the Memory view picks the target up
// (immediately if mounted, otherwise on mount via the pending slot).

export const BROWSE_EVENT = "memento:browse-source";
let pendingBrowse: BrowseTarget | null = null;

export function requestBrowse(target: BrowseTarget) {
  pendingBrowse = target;
  window.dispatchEvent(new CustomEvent(BROWSE_EVENT));
}

export function takePendingBrowse(): BrowseTarget | null {
  const target = pendingBrowse;
  pendingBrowse = null;
  return target;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 100 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}
