import { asciiBytes, Sub } from "@native-sdk/core";
import { type TextInputEvent } from "@native-sdk/core/text";

export interface Memory {
  readonly id: number;
  readonly source: Uint8Array;
  readonly title: Uint8Array;
  readonly preview: Uint8Array;
  readonly age: Uint8Array;
  readonly isMeeting: boolean;
  readonly openLoop: boolean;
}

export type Filter = "all" | "open" | "meetings";
export type Source = "slack" | "notion" | "email" | "meeting" | "note";

export interface Model {
  readonly memories: readonly Memory[];
  readonly visible: readonly Memory[];
  readonly selectedId: number;
  readonly filter: Filter;
  readonly paused: boolean;
  readonly lastSyncMs: number;
  readonly selectedSource: Uint8Array;
  readonly selectedTitle: Uint8Array;
  readonly selectedPreview: Uint8Array;
  readonly selectedAge: Uint8Array;
  readonly selectedOpenLoop: boolean;
  readonly selectedIsMeeting: boolean;
  readonly addOpen: boolean;
  readonly draft: Uint8Array;
  readonly sourcePick: Source;
  readonly openLoopDraft: boolean;
  readonly nextId: number;
}

export type Msg =
  | { readonly kind: "show_all" }
  | { readonly kind: "show_open" }
  | { readonly kind: "show_meetings" }
  | { readonly kind: "select"; readonly id: number }
  | { readonly kind: "toggle_paused" }
  | { readonly kind: "sync" }
  | { readonly kind: "open_add" }
  | { readonly kind: "close_add" }
  | { readonly kind: "draft_edit"; readonly edit: TextInputEvent }
  | { readonly kind: "pick_slack" }
  | { readonly kind: "pick_notion" }
  | { readonly kind: "pick_email" }
  | { readonly kind: "pick_meeting" }
  | { readonly kind: "pick_note" }
  | { readonly kind: "toggle_open_loop" }
  | { readonly kind: "add_memory" };

export const viewUnbound = ["nextId", "lastSyncMs"] as const;

function appendBytes(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length + b.length);
  for (let i = 0; i < a.length; i++) out[i] = a[i];
  for (let j = 0; j < b.length; j++) out[a.length + j] = b[j];
  return out;
}

const seedMemories: Memory[] = [
  { id: 1, source: asciiBytes("Slack"), title: asciiBytes("Product launch thread"), preview: asciiBytes("Jay asked for the final onboarding screenshots before Friday."), age: asciiBytes("12 min ago"), isMeeting: false, openLoop: true },
  { id: 2, source: asciiBytes("Notion"), title: asciiBytes("Q3 planning notes"), preview: asciiBytes("The experiment should measure activation, not raw sign-ups."), age: asciiBytes("48 min ago"), isMeeting: false, openLoop: false },
  { id: 3, source: asciiBytes("Google Meet"), title: asciiBytes("Design review"), preview: asciiBytes("Decided to keep the compact capture indicator in the menu bar."), age: asciiBytes("2 hr ago"), isMeeting: true, openLoop: true },
  { id: 4, source: asciiBytes("Gmail"), title: asciiBytes("Intro to Priya"), preview: asciiBytes("A reply is still needed with two times for next week."), age: asciiBytes("Yesterday"), isMeeting: false, openLoop: true },
];

function findMemory(memories: readonly Memory[], id: number): Memory | null {
  for (const memory of memories) {
    if (memory.id === id) return memory;
  }
  return null;
}

function applyDraftEvent(draft: Uint8Array, edit: TextInputEvent): Uint8Array {
  if (edit.kind === "insert_text") return appendBytes(draft, edit.text);
  if (edit.kind === "delete_backward") {
    return draft.length === 0 ? draft : draft.slice(0, draft.length - 1);
  }
  if (edit.kind === "clear") return asciiBytes("");
  return draft;
}

function byFilter(memories: readonly Memory[], filter: Filter): readonly Memory[] {
  if (filter === "open") return memories.filter((memory) => memory.openLoop);
  if (filter === "meetings") return memories.filter((memory) => memory.isMeeting);
  return memories;
}

function selectMemory(model: Model, memory: Memory): Model {
  return {
    ...model,
    selectedId: memory.id,
    selectedSource: memory.source,
    selectedTitle: memory.title,
    selectedPreview: memory.preview,
    selectedAge: memory.age,
    selectedOpenLoop: memory.openLoop,
    selectedIsMeeting: memory.isMeeting,
  };
}

function modelWithFilter(model: Model, filter: Filter): Model {
  const visible = byFilter(model.memories, filter);
  const base = { ...model, filter, visible };
  return visible.length > 0 ? selectMemory(base, visible[0]) : { ...base, selectedId: -1 };
}

export function initialModel(): Model {
  const model = {
    memories: seedMemories,
    visible: seedMemories,
    selectedId: -1,
    filter: "all",
    paused: false,
    lastSyncMs: -1,
    selectedSource: asciiBytes(""),
    selectedTitle: asciiBytes(""),
    selectedPreview: asciiBytes(""),
    selectedAge: asciiBytes(""),
    selectedOpenLoop: false,
    selectedIsMeeting: false,
    addOpen: false,
    draft: asciiBytes(""),
    sourcePick: "note",
    openLoopDraft: false,
    nextId: 100,
  } as Model;
  return seedMemories.length > 0 ? selectMemory(model, seedMemories[0]) : model;
}

export function memoryCount(model: Model): number {
  return model.memories.length;
}

export function openLoopCount(model: Model): number {
  let n = 0;
  for (const memory of model.memories) {
    if (memory.openLoop) n += 1;
  }
  return n;
}

export function meetingCount(model: Model): number {
  let n = 0;
  for (const memory of model.memories) {
    if (memory.isMeeting) n += 1;
  }
  return n;
}

export function hasDraft(model: Model): boolean {
  return model.draft.length > 0;
}

export function isEmpty(model: Model): boolean {
  return model.visible.length === 0;
}

export function hasSelection(model: Model): boolean {
  return model.selectedId >= 0;
}

export function pickedSourceLabel(model: Model): Uint8Array {
  if (model.sourcePick === "slack") return asciiBytes("Slack");
  if (model.sourcePick === "notion") return asciiBytes("Notion");
  if (model.sourcePick === "email") return asciiBytes("Email");
  if (model.sourcePick === "meeting") return asciiBytes("Meeting");
  return asciiBytes("Note");
}

export function syncedLabel(model: Model): Uint8Array {
  if (model.lastSyncMs < 0) return asciiBytes("Not synced yet");
  return asciiBytes("Synced");
}

export function update(model: Model, msg: Msg): Model {
  switch (msg.kind) {
    case "show_all":
      return modelWithFilter(model, "all");
    case "show_open":
      return modelWithFilter(model, "open");
    case "show_meetings":
      return modelWithFilter(model, "meetings");
    case "select": {
      const memory = findMemory(model.memories, msg.id);
      return memory === null ? model : selectMemory(model, memory);
    }
    case "toggle_paused":
      return { ...model, paused: !model.paused };
    case "sync":
      return { ...model, lastSyncMs: 0 };
    case "open_add":
      return { ...model, addOpen: true };
    case "close_add":
      return { ...model, addOpen: false, draft: asciiBytes("") };
    case "draft_edit":
      return { ...model, draft: applyDraftEvent(model.draft, msg.edit) };
    case "pick_slack":
      return { ...model, sourcePick: "slack" };
    case "pick_notion":
      return { ...model, sourcePick: "notion" };
    case "pick_email":
      return { ...model, sourcePick: "email" };
    case "pick_meeting":
      return { ...model, sourcePick: "meeting" };
    case "pick_note":
      return { ...model, sourcePick: "note" };
    case "toggle_open_loop":
      return { ...model, openLoopDraft: !model.openLoopDraft };
    case "add_memory": {
      if (model.draft.length === 0) return model;
      const memory: Memory = {
        id: model.nextId,
        source: pickedSourceLabel(model),
        title: model.draft,
        preview: asciiBytes("Saved from the composer"),
        age: asciiBytes("just now"),
        isMeeting: model.sourcePick === "meeting",
        openLoop: model.openLoopDraft,
      };
      const memories: readonly Memory[] = [...model.memories, memory];
      const visible = byFilter(memories, model.filter);
      const next: Model = {
        ...model,
        memories,
        visible,
        nextId: model.nextId < 1000000 ? model.nextId + 1 : model.nextId,
        addOpen: false,
        draft: asciiBytes(""),
        sourcePick: "note" as Source,
        openLoopDraft: false,
      };
      return selectMemory(next, memory);
    }
  }
}

export function subscriptions(_model: Model): Sub<Msg> {
  return Sub.none;
}
