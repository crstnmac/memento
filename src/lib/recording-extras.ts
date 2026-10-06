import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { api } from "./api";

export interface UntranscribedRecording {
  id: number;
  title: string;
  startedAt: number;
  durationMs: number | null;
}

export interface RecordingWarning {
  conversationId: number;
  message: string;
}

export const listUntranscribedRecordings = () =>
  invoke<UntranscribedRecording[]>("list_untranscribed_recordings");

export function onRecordingWarning(handler: (w: RecordingWarning) => void): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;
  void listen<RecordingWarning>("recording-warning", (e) => handler(e.payload)).then((fn) => {
    if (disposed) fn();
    else unlisten = fn;
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}

/** Start transcription for one recording and resolve once it finishes or fails. */
export async function transcribeAndWait(conversationId: number): Promise<void> {
  const unlistens: Array<() => void> = [];
  const done = new Promise<void>((resolve) => {
    const settle = (e: { payload: { conversationId: number } }) => {
      if (e.payload.conversationId === conversationId) resolve();
    };
    void Promise.all([
      listen<{ conversationId: number }>("transcription-finished", settle),
      listen<{ conversationId: number }>("transcription-failed", settle),
    ]).then((fns) => unlistens.push(...fns));
  });
  try {
    await api.transcribeConversation(conversationId);
    await done;
  } finally {
    unlistens.forEach((fn) => fn());
  }
}
