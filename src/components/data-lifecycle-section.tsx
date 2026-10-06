import { useCallback, useEffect, useRef, useState } from "react";
import { Trash2Icon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { formatDate, timeAgo } from "@/lib/format";
import {
  browseTargetFor,
  filterForSource,
  formatBytes,
  lifecycle,
  requestBrowse,
  type CaptureSource,
  type CaptureSummary,
  type DeleteFilter,
  type DeleteSummary,
} from "@/lib/lifecycle";
import { useMemento } from "@/lib/store";

const nativeRuntime = "__TAURI_INTERNALS__" in window;

const CATEGORY_LABELS: Record<string, string> = {
  chat: "Chat",
  email: "Email",
  meeting: "Meetings",
  productivity: "Notes & tasks",
  notes: "Written by you",
  other: "Other apps",
};

type RangeId = "hour" | "today" | "week" | "all";
const RANGES: { id: RangeId; label: string }[] = [
  { id: "hour", label: "Last hour" },
  { id: "today", label: "Today" },
  { id: "week", label: "Last 7 days" },
  { id: "all", label: "All time" },
];

/** Start of the range, or undefined for all time. Computed once per choice so
 *  the preview and the delete cover exactly the same moments. */
function rangeStart(range: RangeId, now: number): number | undefined {
  if (range === "hour") return now - 60 * 60 * 1000;
  if (range === "week") return now - 7 * 24 * 60 * 60 * 1000;
  if (range === "today") {
    const start = new Date(now);
    start.setHours(0, 0, 0, 0);
    return start.getTime();
  }
  return undefined;
}

const plural = (count: number, one: string, many = `${one}s`) => `${count.toLocaleString()} ${count === 1 ? one : many}`;

function describe(summary: DeleteSummary): string {
  const parts: string[] = [plural(summary.memories, "memory", "memories")];
  if (summary.followUps > 0) parts.push(plural(summary.followUps, "follow-up"));
  if (summary.recordings > 0) parts.push(plural(summary.recordings, "recording"));
  const list = parts.length > 1 ? `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}` : parts[0];
  return `${list}${summary.vaultFiles > 0 ? ` and ${plural(summary.vaultFiles, "Markdown file")}` : ""}`;
}

interface DialogState {
  /** Source id to limit to, or "" for all sources. */
  scope: string;
  range: RangeId;
  includeNotes: boolean;
  includeRecordings: boolean;
}

export function DataLifecycleSection() {
  const { refreshAll } = useMemento();
  const [summary, setSummary] = useState<CaptureSummary | null>(null);
  const [loading, setLoading] = useState(nativeRuntime);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [dialog, setDialog] = useState<DialogState | null>(null);

  const load = useCallback(async () => {
    if (!nativeRuntime) return;
    try {
      setSummary(await lifecycle.getCaptureSummary());
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const sources = summary?.sources ?? [];

  return <section aria-labelledby="data-lifecycle-title" className="settings-section">
    <div className="mb-3 flex items-start justify-between gap-4">
      <div>
        <h2 id="data-lifecycle-title" className="text-sm font-semibold">What Memento has captured</h2>
        <p className="mt-1 text-xs leading-5 text-muted-foreground">See what is stored from each app, and remove it whenever you like.</p>
      </div>
      <Button variant="outline" size="sm" onClick={() => setDialog({ scope: "", range: "hour", includeNotes: false, includeRecordings: false })} disabled={sources.length === 0}>
        <Trash2Icon aria-hidden="true" /> Delete a time range…
      </Button>
    </div>

    <div className="settings-group">
      {loading ? <p className="px-4 py-5 text-center text-xs text-muted-foreground" role="status">Loading…</p>
        : sources.length === 0 ? <p className="px-4 py-5 text-center text-xs text-muted-foreground">{error ? "Couldn't load this list." : "Nothing has been captured yet."}</p>
        : <ul className="max-h-80 divide-y overflow-y-auto">
          {sources.map((source) => <li key={source.id} className="flex min-h-14 items-center gap-3 px-4 py-2">
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm">{source.name}</p>
              <p className="text-xs leading-5 text-muted-foreground">
                {CATEGORY_LABELS[source.category] ?? "Other apps"} · {source.lastAt ? `last captured ${timeAgo(source.lastAt)}` : "never captured"} · about {formatBytes(source.bytes)}
              </p>
            </div>
            <span className="shrink-0 text-xs tabular-nums text-muted-foreground">{plural(source.memories, "memory", "memories")}</span>
            <Button variant="ghost" size="sm" aria-label={`View memories from ${source.name}`} onClick={() => requestBrowse(browseTargetFor(source))}>View</Button>
            <Button variant="ghost" size="sm" aria-label={`Delete memories from ${source.name}…`} onClick={() => setDialog({ scope: source.id, range: "all", includeNotes: false, includeRecordings: false })}>Delete…</Button>
          </li>)}
        </ul>}
      {summary && sources.length > 0 ? <p className="border-t px-4 py-2 text-xs text-muted-foreground">
        {plural(summary.totals.memories, "memory", "memories")}
        {summary.totals.firstAt ? ` since ${formatDate(summary.totals.firstAt)}` : ""}
        {" · "}Database {formatBytes(summary.disk.databaseBytes)}
        {" · "}Recordings {formatBytes(summary.disk.recordingsBytes)}
        {" · "}Memory folder {formatBytes(summary.disk.vaultBytes)}
      </p> : null}
    </div>
    {notice ? <p role="status" className="mt-2 text-xs text-muted-foreground">{notice}</p> : null}
    {error ? <p role="alert" className="mt-2 text-xs text-destructive">{error}</p> : null}

    {dialog ? <DeleteDialog
      key={dialog.scope + dialog.range}
      initial={dialog}
      sources={sources}
      onClose={() => setDialog(null)}
      onDeleted={async (result) => {
        setDialog(null);
        setNotice(`Deleted ${describe(result)}.${result.failedFiles > 0 ? ` ${plural(result.failedFiles, "file")} couldn't be removed.` : ""}`);
        await Promise.all([refreshAll(), load()]);
      }}
    /> : null}
  </section>;
}

function DeleteDialog({ initial, sources, onClose, onDeleted }: {
  initial: DialogState;
  sources: CaptureSource[];
  onClose: () => void;
  onDeleted: (result: DeleteSummary) => Promise<void>;
}) {
  const [state, setState] = useState(initial);
  // Fixed per choice of range; also caps the range at "now" so new captures
  // arriving while the dialog is open are not deleted unseen.
  const [bounds, setBounds] = useState(() => ({ now: Date.now(), since: rangeStart(initial.range, Date.now()) }));
  const [preview, setPreview] = useState<DeleteSummary | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const cancel = useRef<HTMLButtonElement>(null);

  const scoped = sources.find((source) => source.id === state.scope);
  const hasRecordings = state.scope === "" ? sources.some((s) => s.id === "meeting" || s.id === "voice-note") : state.scope === "meeting" || state.scope === "voice-note";
  const hasNotes = state.scope === "" && sources.some((s) => s.kind === "notes");

  const filter: DeleteFilter = {
    ...(scoped ? filterForSource(scoped) : { all: true }),
    sinceMs: bounds.since,
    untilMs: bounds.now + 1,
    includeNotes: scoped?.kind === "notes" || state.includeNotes,
    includeRecordings: hasRecordings && state.includeRecordings,
  };
  const filterKey = JSON.stringify(filter);

  useEffect(() => {
    let cancelled = false;
    setPreview(null);
    setPreviewError(null);
    lifecycle.previewDelete(JSON.parse(filterKey) as DeleteFilter)
      .then((result) => { if (!cancelled) setPreview(result); })
      .catch((cause) => { if (!cancelled) setPreviewError(cause instanceof Error ? cause.message : String(cause)); });
    return () => { cancelled = true; };
  }, [filterKey]);

  function chooseRange(range: RangeId) {
    const now = Date.now();
    setBounds({ now, since: rangeStart(range, now) });
    setState((previous) => ({ ...previous, range }));
  }

  async function confirm() {
    setBusy(true);
    setFailure(null);
    let result: DeleteSummary;
    try {
      result = await lifecycle.deleteMemories(filter);
    } catch (cause) {
      setFailure(cause instanceof Error ? cause.message : String(cause));
      setBusy(false);
      return;
    }
    await onDeleted(result);
  }

  const nothing = preview !== null && preview.memories === 0 && preview.recordings === 0;
  const label = scoped ? scoped.name : "all sources";

  return <Dialog open onOpenChange={(open) => { if (!open && !busy) onClose(); }}>
    <DialogContent initialFocus={cancel} className="sm:max-w-md">
      <DialogHeader>
        <DialogTitle>Delete captured memories</DialogTitle>
        <DialogDescription>Choose what to remove. Nothing is deleted until you confirm.</DialogDescription>
      </DialogHeader>

      <div className="space-y-4">
        <div className="space-y-1.5">
          <label htmlFor="delete-scope" className="text-xs font-medium">From</label>
          <select
            id="delete-scope"
            value={state.scope}
            onChange={(event) => setState({ ...state, scope: event.target.value, includeRecordings: false, includeNotes: false })}
            className="h-8 w-full rounded-lg border border-input bg-background px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
          >
            <option value="">All sources</option>
            {sources.map((source) => <option key={source.id} value={source.id}>{source.name} ({source.memories.toLocaleString()})</option>)}
          </select>
        </div>

        <fieldset className="space-y-1.5">
          <legend className="text-xs font-medium">Time range</legend>
          <div className="flex flex-wrap gap-1.5">
            {RANGES.map((range) => <Button key={range.id} type="button" size="sm" variant={state.range === range.id ? "default" : "outline"} aria-pressed={state.range === range.id} onClick={() => chooseRange(range.id)}>{range.label}</Button>)}
          </div>
        </fieldset>

        {hasNotes ? <label className="flex items-start gap-2 text-sm"><input type="checkbox" className="mt-0.5 size-4" checked={state.includeNotes} onChange={(event) => setState({ ...state, includeNotes: event.target.checked })} /><span>Also delete notes I wrote myself</span></label> : null}
        {hasRecordings ? <label className="flex items-start gap-2 text-sm"><input type="checkbox" className="mt-0.5 size-4" checked={state.includeRecordings} onChange={(event) => setState({ ...state, includeRecordings: event.target.checked })} /><span>Also delete meeting and voice note recordings (audio files)</span></label> : null}

        <div role="status" aria-live="polite" className="rounded-lg border bg-muted/40 px-3 py-2.5 text-sm leading-6">
          {previewError ? <span className="text-destructive">Couldn't check what would be deleted. {previewError}</span>
            : preview === null ? <span className="text-muted-foreground">Checking what would be deleted…</span>
            : nothing ? <span>Nothing from {label} matches this time range.</span>
            : <>
              <span>This permanently deletes {describe(preview)}. Follow-ups saved from these memories are deleted too. This can't be undone.</span>
              {preview.bytesFreed.total > 0 ? <span className="mt-1 block text-xs text-muted-foreground">Frees about {formatBytes(preview.bytesFreed.total)}.</span> : null}
            </>}
        </div>
        {failure ? <p role="alert" className="text-xs text-destructive">Couldn't delete: {failure}</p> : null}
      </div>

      <DialogFooter>
        <Button ref={cancel} variant="outline" onClick={onClose} disabled={busy}>Cancel</Button>
        <Button variant="destructive" onClick={() => void confirm()} disabled={busy || preview === null || nothing || previewError !== null}>
          {busy ? "Deleting…" : preview && !nothing ? preview.memories > 0 ? `Delete ${plural(preview.memories, "memory", "memories")}` : `Delete ${plural(preview.recordings, "recording")}` : "Delete"}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>;
}
