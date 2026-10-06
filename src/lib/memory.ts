export type Filter = "all" | "open" | "meetings";
export type Source = "slack" | "notion" | "email" | "meeting" | "note";

export interface Memory {
  readonly id: number;
  readonly source: string;
  readonly title: string;
  readonly preview: string;
  readonly age: string;
  readonly isMeeting: boolean;
  readonly openLoop: boolean;
}

export interface MemoryState {
  readonly memories: readonly Memory[];
  readonly visible: readonly Memory[];
  readonly selectedId: number;
  readonly filter: Filter;
  readonly paused: boolean;
  readonly lastSyncMs: number;
  readonly addOpen: boolean;
  readonly draft: string;
  readonly sourcePick: Source;
  readonly openLoopDraft: boolean;
  readonly nextId: number;
}

export type MemoryAction =
  | { readonly kind: "show_all" }
  | { readonly kind: "show_open" }
  | { readonly kind: "show_meetings" }
  | { readonly kind: "select"; readonly id: number }
  | { readonly kind: "toggle_paused" }
  | { readonly kind: "sync" }
  | { readonly kind: "open_add" }
  | { readonly kind: "close_add" }
  | { readonly kind: "draft_edit"; readonly text: string }
  | { readonly kind: "pick_source"; readonly source: Source }
  | { readonly kind: "toggle_open_loop" }
  | { readonly kind: "add_memory" };

const seedMemories: readonly Memory[] = [
  { id: 1, source: "Slack", title: "Product launch thread", preview: "Jay asked for the final onboarding screenshots before Friday.", age: "12 min ago", isMeeting: false, openLoop: true },
  { id: 2, source: "Notion", title: "Q3 planning notes", preview: "The experiment should measure activation, not raw sign-ups.", age: "48 min ago", isMeeting: false, openLoop: false },
  { id: 3, source: "Google Meet", title: "Design review", preview: "Decided to keep the compact capture indicator in the menu bar.", age: "2 hr ago", isMeeting: true, openLoop: true },
  { id: 4, source: "Gmail", title: "Intro to Priya", preview: "A reply is still needed with two times for next week.", age: "Yesterday", isMeeting: false, openLoop: true },
];

function findMemory(memories: readonly Memory[], id: number): Memory | null {
  return memories.find((memory) => memory.id === id) ?? null;
}

function byFilter(memories: readonly Memory[], filter: Filter): readonly Memory[] {
  if (filter === "open") return memories.filter((memory) => memory.openLoop);
  if (filter === "meetings") return memories.filter((memory) => memory.isMeeting);
  return memories;
}

function modelWithFilter(state: MemoryState, filter: Filter): MemoryState {
  const visible = byFilter(state.memories, filter);
  const first = visible[0] ?? null;
  return {
    ...state,
    filter,
    visible,
    selectedId: first === null ? -1 : first.id,
  };
}

export function initialMemoryState(): MemoryState {
  const state: MemoryState = {
    memories: seedMemories,
    visible: seedMemories,
    selectedId: -1,
    filter: "all",
    paused: false,
    lastSyncMs: -1,
    addOpen: false,
    draft: "",
    sourcePick: "note",
    openLoopDraft: false,
    nextId: 100,
  };
  return seedMemories.length > 0 ? modelWithFilter(state, "all") : state;
}

export function pickedSourceLabel(source: Source): string {
  switch (source) {
    case "slack":
      return "Slack";
    case "notion":
      return "Notion";
    case "email":
      return "Email";
    case "meeting":
      return "Meeting";
    case "note":
      return "Note";
  }
}

export function selectedMemory(state: MemoryState): Memory | null {
  return findMemory(state.memories, state.selectedId);
}

export function memoryCount(state: MemoryState): number {
  return state.memories.length;
}

export function openLoopCount(state: MemoryState): number {
  let n = 0;
  for (const memory of state.memories) {
    if (memory.openLoop) n += 1;
  }
  return n;
}

export function meetingCount(state: MemoryState): number {
  let n = 0;
  for (const memory of state.memories) {
    if (memory.isMeeting) n += 1;
  }
  return n;
}

export function hasDraft(state: MemoryState): boolean {
  return state.draft.length > 0;
}

export function isEmpty(state: MemoryState): boolean {
  return state.visible.length === 0;
}

export function hasSelection(state: MemoryState): boolean {
  return state.selectedId >= 0;
}

export function syncedLabel(state: MemoryState): string {
  return state.lastSyncMs < 0 ? "Not synced yet" : "Synced";
}

export function memoryReducer(state: MemoryState, action: MemoryAction): MemoryState {
  switch (action.kind) {
    case "show_all":
      return modelWithFilter(state, "all");
    case "show_open":
      return modelWithFilter(state, "open");
    case "show_meetings":
      return modelWithFilter(state, "meetings");
    case "select": {
      const memory = findMemory(state.memories, action.id);
      return memory === null ? state : { ...state, selectedId: memory.id };
    }
    case "toggle_paused":
      return { ...state, paused: !state.paused };
    case "sync":
      return { ...state, lastSyncMs: Date.now() };
    case "open_add":
      return { ...state, addOpen: true };
    case "close_add":
      return { ...state, addOpen: false, draft: "" };
    case "draft_edit":
      return { ...state, draft: action.text };
    case "pick_source":
      return { ...state, sourcePick: action.source };
    case "toggle_open_loop":
      return { ...state, openLoopDraft: !state.openLoopDraft };
    case "add_memory": {
      if (state.draft.length === 0) return state;
      const memory: Memory = {
        id: state.nextId,
        source: pickedSourceLabel(state.sourcePick),
        title: state.draft,
        preview: "Saved from the composer",
        age: "just now",
        isMeeting: state.sourcePick === "meeting",
        openLoop: state.openLoopDraft,
      };
      const memories: readonly Memory[] = [...state.memories, memory];
      const next: MemoryState = {
        ...state,
        memories,
        nextId: state.nextId < 1000000 ? state.nextId + 1 : state.nextId,
        addOpen: false,
        draft: "",
        sourcePick: "note",
        openLoopDraft: false,
      };
      return modelWithFilter(next, next.filter);
    }
  }
}
