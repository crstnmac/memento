import { useCallback, useEffect, useMemo, useState } from "react";
import { ArrowLeftIcon, SearchIcon, XIcon } from "lucide-react";

import { MemoryDetail } from "@/components/memory-detail";
import { MemoryList } from "@/components/memory-list";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { BROWSE_EVENT, browseTargetFor, lifecycle, takePendingBrowse, type BrowseTarget, type CaptureSource } from "@/lib/lifecycle";
import { useMemento } from "@/lib/store";
import { cn } from "@/lib/utils";
import type { MemoryRow } from "@/lib/types";

const PAGE_SIZE = 200;
const nativeRuntime = "__TAURI_INTERNALS__" in window;
const sameTarget = (a: BrowseTarget, b: BrowseTarget) => a.sourceId === b.sourceId && a.app === b.app;

export function TimelineView({ initialSelectedId = null, initialQuery = "" }: { initialSelectedId?: number | null; initialQuery?: string }) {
  const { memories, refreshMemories } = useMemento();
  const [query, setQuery] = useState(initialQuery);
  useEffect(() => { if (initialQuery) setQuery(initialQuery); }, [initialQuery]);
  const [selectedId, setSelectedId] = useState<number | null>(initialSelectedId);
  const [busy, setBusy] = useState(false);
  const [searchResults, setSearchResults] = useState<MemoryRow[] | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);

  // Browse by source ("what was captured from Slack"). Replaces the recent list
  // while active; typing a search clears it and picking a source clears the search.
  const [sources, setSources] = useState<CaptureSource[]>([]);
  const [filter, setFilter] = useState<BrowseTarget | null>(null);
  const [filtered, setFiltered] = useState<MemoryRow[] | null>(null);
  const [filterBusy, setFilterBusy] = useState(false);
  const [filterError, setFilterError] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(false);

  const loadSources = useCallback(() => {
    if (!nativeRuntime) return;
    lifecycle.getCaptureSummary().then((summary) => setSources(summary.sources)).catch(console.error);
  }, []);
  useEffect(loadSources, [loadSources]);
  useEffect(() => {
    // Arrived from Settings ("View" on a source) while this view was not mounted.
    const pending = takePendingBrowse();
    if (pending) setFilter(pending);
    const onBrowse = () => {
      const target = takePendingBrowse();
      if (target) { setFilter(target); setQuery(""); setSelectedId(null); }
    };
    window.addEventListener(BROWSE_EVENT, onBrowse);
    return () => window.removeEventListener(BROWSE_EVENT, onBrowse);
  }, []);
  const activeSource = filter ? sources.find((source) => sameTarget(browseTargetFor(source), filter)) : undefined;
  const filterLabel = activeSource?.name ?? filter?.app ?? filter?.sourceId ?? "";

  useEffect(() => {
    if (!filter) { setFiltered(null); setFilterError(null); setHasMore(false); return; }
    let cancelled = false;
    setFilterBusy(true);
    setFilterError(null);
    lifecycle.listMemoriesFiltered(filter, {}, PAGE_SIZE, 0)
      .then((rows) => { if (!cancelled) { setFiltered(rows); setHasMore(rows.length === PAGE_SIZE); } })
      .catch((e) => { console.error(e); if (!cancelled) setFilterError("Couldn't load memories from this source."); })
      .finally(() => { if (!cancelled) setFilterBusy(false); });
    return () => { cancelled = true; };
  }, [filter]);

  async function loadMore() {
    if (!filter || !filtered) return;
    try {
      const rows = await lifecycle.listMemoriesFiltered(filter, {}, PAGE_SIZE, filtered.length);
      setFiltered((previous) => [...(previous ?? []), ...rows]);
      setHasMore(rows.length === PAGE_SIZE);
    } catch (e) {
      console.error(e);
      setFilterError("Couldn't load more memories.");
    }
  }

  function chooseSource(target: BrowseTarget | null) {
    setFilter(target);
    setQuery("");
    setSelectedId(null);
  }

  useEffect(() => { if (initialSelectedId !== null) setSelectedId(initialSelectedId); }, [initialSelectedId]);

  const moments = useMemo(() => searchResults ?? (filter ? filtered ?? [] : memories), [searchResults, filter, filtered, memories]);

  const selected = moments.find((m) => m.id === selectedId) ?? null;

  useEffect(() => {
    if (selectedId !== null && !moments.some((m) => m.id === selectedId)) {
      setSelectedId(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [moments]);

  useEffect(() => {
    let cancelled = false;
    const q = query.trim();
    if (q.length === 0) {
      setSearchResults(null);
      setBusy(false);
      return;
    }
    setBusy(true);
    setSearchError(null);
    (async () => {
      try {
        const result = await api.searchText(q);
        if (!cancelled) setSearchResults(result);
      } catch (e) {
        console.error(e);
        if (!cancelled) setSearchError("Search is unavailable right now. Your timeline is still here.");
      } finally {
        if (!cancelled) setBusy(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [query]);

  function updateQuery(value: string) {
    setQuery(value);
    setSelectedId(null);
    if (value.trim()) setFilter(null);
  }

  async function handleDelete(id: number) {
    await api.deleteMemory(id);
    await refreshMemories();
    if (selectedId === id) setSelectedId(null);
    if (searchResults) {
      setSearchResults((prev) => (prev ? prev.filter((m) => m.id !== id) : prev));
    }
    setFiltered((prev) => (prev ? prev.filter((m) => m.id !== id) : prev));
    loadSources();
  }

  return (
    <div className="workspace-page">
      <div className="workspace-content flex min-h-0 flex-1 flex-col overflow-hidden">
      <header className="workspace-header">
        <div>
          <h1 className="workspace-title">Search</h1>
          <p className="workspace-subtitle">Find something from your workday.</p>
        </div>
      </header>
      <div className="flex shrink-0 items-center gap-2 border-b py-5">
        <div className="relative min-w-0 flex-1">
          <SearchIcon className="absolute left-3.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            id="memory-search"
            value={query}
            onChange={(e) => updateQuery(e.target.value)}
            placeholder="What are you trying to remember?"
            className="h-12 bg-background pl-10 pr-10 text-sm shadow-none"
          />
          {query ? (
            <button type="button" aria-label="Clear search" onClick={() => updateQuery("")} className="absolute right-2 top-1/2 flex size-8 -translate-y-1/2 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40">
              <XIcon className="size-4" />
            </button>
          ) : null}
        </div>
      </div>

      {sources.length > 0 ? (
        <div role="group" aria-label="Filter by source" className="flex max-h-24 shrink-0 flex-wrap gap-1.5 overflow-y-auto border-b py-3">
          <SourceChip label="All sources" active={!filter} onClick={() => chooseSource(null)} />
          {sources.map((source) => (
            <SourceChip key={source.id} label={source.name} count={source.memories} active={!!filter && sameTarget(browseTargetFor(source), filter)} onClick={() => chooseSource(browseTargetFor(source))} />
          ))}
        </div>
      ) : null}

      {filterError ? <p role="alert" className="border-b border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">{filterError}</p> : null}
      {searchError ? <p role="alert" className="border-b border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">{searchError}</p> : null}

      {busy || filterBusy ? (
        <div className="py-8 text-sm text-muted-foreground" role="status">{busy ? "Searching…" : "Loading…"}</div>
      ) : moments.length === 0 ? (
        <div className="flex min-h-0 flex-1 items-center justify-center text-center">
          <div className="max-w-sm">
            <h2 className="text-sm font-semibold">{query.trim() ? "No memories found" : filter ? `Nothing from ${filterLabel}` : "Nothing here yet"}</h2>
            <p className="mt-2 text-sm leading-6 text-muted-foreground">
              {query.trim() ? "Try a different word or phrase." : "Your captured work, meetings, and notes will appear here."}
            </p>
          </div>
        </div>
      ) : selected ? (
        <section
          aria-label="Memory detail"
          className="flex min-h-0 flex-1 flex-col pt-5"
        >
          <Button variant="ghost" size="sm" className="mb-5 w-fit -ml-2 text-muted-foreground" onClick={() => setSelectedId(null)}>
            <ArrowLeftIcon /> Back to {query.trim() ? "results" : filter ? filterLabel : "recent"}
          </Button>
          <MemoryDetail memory={selected} onDelete={handleDelete} />
        </section>
      ) : (
        <section aria-label="Memory list" className="flex min-h-0 flex-1 flex-col overflow-hidden pt-4">
          <div className="flex items-baseline justify-between gap-4 px-1 pb-2">
            <h2 className="workspace-section-heading">{query.trim() ? "Results" : filter ? `From ${filterLabel}` : "Recent"}</h2>
            <span className="text-xs tabular-nums text-muted-foreground">{moments.length}</span>
          </div>
          <MemoryList
            memories={moments}
            selectedId={-1}
            empty={moments.length === 0}
            onSelect={setSelectedId}
            onLoadMore={filter && hasMore && !query.trim() ? () => void loadMore() : undefined}
          />
        </section>
      )}
      </div>
    </div>
  );
}

function SourceChip({ label, count, active, onClick }: { label: string; count?: number; active: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "inline-flex h-7 items-center gap-1.5 rounded-full border px-3 text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring/50",
        active ? "border-primary bg-primary text-primary-foreground" : "bg-background text-muted-foreground hover:bg-muted hover:text-foreground",
      )}
    >
      {label}
      {count !== undefined ? <span className="tabular-nums opacity-70">{count.toLocaleString()}</span> : null}
    </button>
  );
}
