import { useEffect, useMemo, useState } from "react";
import {
  CheckIcon,
  InfoIcon,
  XIcon,
  Loader2Icon,
  MicIcon,
  RadioIcon,
  StickyNoteIcon,
  Trash2Icon,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { ScrollArea } from "@/components/ui/scroll-area";
import { api } from "@/lib/api";
import { formatClock, formatDate, formatDuration, formatTime } from "@/lib/format";
import {
  listUntranscribedRecordings,
  onRecordingWarning,
  transcribeAndWait,
  type UntranscribedRecording,
} from "@/lib/recording-extras";
import { useMemento } from "@/lib/store";
import type { Conversation, RecordingMode } from "@/lib/types";

// Ask for Screen Recording at most once per launch. macOS only applies a new
// grant after a relaunch, so re-opening System Settings on every recording
// would just nag someone who chose to record the microphone only.
let askedForScreenRecording = false;

export function MeetingsView() {
  const {
    conversations,
    status,
    settings,
    saveSetting,
    transcribing,
    transcriptionErrors,
    refreshAll,
  } = useMemento();
  const [starting, setStarting] = useState(false);
  const [startedMs, setStartedMs] = useState<number | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [confirmDelete, setConfirmDelete] = useState<number | null>(null);
  const [recordingError, setRecordingError] = useState<string | null>(null);

  const recording = status?.recording ?? false;
  const [warnings, setWarnings] = useState<Array<{ id: number; message: string }>>([]);
  const [pending, setPending] = useState<UntranscribedRecording[]>([]);
  const [batch, setBatch] = useState<{ done: number; total: number } | null>(null);
  const batchRunning = batch !== null;

  useEffect(() => {
    let nextId = 0;
    return onRecordingWarning(({ message }) => {
      setWarnings((prev) =>
        prev.some((w) => w.message === message)
          ? prev
          : [...prev, { id: nextId++, message }].slice(-3),
      );
    });
  }, []);

  useEffect(() => {
    if (!recording) setWarnings([]);
  }, [recording]);

  useEffect(() => {
    if (batchRunning) return;
    let cancelled = false;
    listUntranscribedRecordings()
      .then((rows) => {
        if (!cancelled) setPending(rows);
      })
      .catch(() => {
        if (!cancelled) setPending([]);
      });
    return () => {
      cancelled = true;
    };
  }, [conversations, batchRunning]);

  // Recordings already transcribing, or that just failed (they show their own
  // error and retry button), are not offered again.
  const waiting = pending.filter(
    (r) => !transcribing.has(r.id) && !(r.id in transcriptionErrors),
  );

  async function transcribeAll() {
    const queue = waiting.map((r) => r.id);
    setBatch({ done: 0, total: queue.length });
    try {
      for (const [index, id] of queue.entries()) {
        try {
          await transcribeAndWait(id);
        } catch {
          // Failures are reported per recording via transcription-failed.
        }
        setBatch({ done: index + 1, total: queue.length });
      }
    } finally {
      setBatch(null);
      await refreshAll();
    }
  }

  async function waitForPermission(kind: "microphone" | "screenRecording") {
    for (let attempt = 0; attempt < 20; attempt += 1) {
      const permissions = await api.getPermissions();
      if (permissions[kind] === "granted") return permissions;
      if (permissions[kind] === "denied" || permissions[kind] === "restricted") return permissions;
      await new Promise((resolve) => window.setTimeout(resolve, 300));
    }
    return api.getPermissions();
  }

  useEffect(() => {
    if (!recording) {
      // Otherwise the next recording would briefly show the previous one's time.
      setStartedMs(null);
      setElapsed(0);
      return;
    }
    // Use the recording's real start time. Starting from "now" restarted the
    // timer at 0 on every visit to this tab and for recordings started from
    // the menu bar or by auto-start.
    let cancelled = false;
    api
      .recordingStatus()
      .then((active) => { if (!cancelled) setStartedMs(active?.startedMs ?? Date.now()); })
      .catch(() => { if (!cancelled) setStartedMs(Date.now()); });
    return () => { cancelled = true; };
  }, [recording]);

  useEffect(() => {
    if (startedMs === null) return;
    const tick = () => setElapsed(Math.max(0, Date.now() - startedMs));
    tick();
    const timer = setInterval(tick, 250);
    return () => clearInterval(timer);
  }, [startedMs]);

  const sorted = useMemo(
    () => [...conversations].sort((a, b) => b.startedAt - a.startedAt),
    [conversations],
  );

  async function start() {
    setStarting(true);
    setRecordingError(null);
    try {
      const kind = settings.recordingKind === "voice-note" ? "voice-note" : "meeting";
      if (kind === "meeting" && !settings.recordMic && !settings.recordSystem) {
        throw new Error("Enable microphone or system audio in Settings before recording a meeting.");
      }
      const mode = kind === "voice-note"
        ? "mic"
        : settings.recordMic && settings.recordSystem ? "both"
          : settings.recordSystem ? "system" : "mic";
      const needsMic = mode === "mic" || mode === "both";
      const needsSystem = mode === "system" || mode === "both";
      let permissions = await api.getPermissions();
      if (needsMic && permissions.microphone !== "granted") {
        await api.requestPermission("microphone");
        permissions = await waitForPermission("microphone");
      }
      if (needsSystem && permissions.screenRecording !== "granted" && !askedForScreenRecording) {
        askedForScreenRecording = true;
        await api.requestPermission("screen-recording");
        permissions = await waitForPermission("screenRecording");
      }
      permissions = await api.getPermissions();
      if (needsMic && permissions.microphone !== "granted") {
        throw new Error("Microphone access is required. Allow Memento in System Settings → Privacy & Security → Microphone, then try again.");
      }
      let startMode: RecordingMode = mode;
      let notice: string | null = null;
      if (needsSystem && permissions.screenRecording !== "granted") {
        // The microphone works, so don't refuse to record: keep your side of
        // the conversation and say what is missing (same as auto-start does).
        if (mode === "both") {
          startMode = "mic";
          notice = "Recording your microphone only. To also capture the other people on the call, allow Memento in System Settings → Privacy & Security → Screen & System Audio Recording, then reopen Memento.";
        } else {
          throw new Error("System audio requires Screen Recording access. Allow Memento in System Settings → Privacy & Security → Screen & System Audio Recording, then quit and reopen Memento.");
        }
      }
      await api.startRecording(startMode, kind);
      await refreshAll();
      if (notice) {
        const message = notice;
        setWarnings((prev) => [...prev, { id: Date.now(), message }].slice(-3));
      }
    } catch (e) {
      setRecordingError(e instanceof Error ? e.message : String(e));
    } finally {
      setStarting(false);
    }
  }

  async function stop() {
    try {
      await api.stopRecording();
      await refreshAll();
    } catch (e) {
      setRecordingError(e instanceof Error ? e.message : String(e));
    }
  }

  async function removeRecording(conversation: Conversation) {
    try {
      await api.deleteConversation(conversation.id);
      await refreshAll();
    } catch (e) {
      setRecordingError(e instanceof Error ? e.message : String(e));
    } finally {
      setConfirmDelete(null);
    }
  }

  async function transcribe(conversation: Conversation) {
    setRecordingError(null);
    try {
      // Runs in the background on the Rust side; progress arrives via the
      // `transcription-started` / `transcription-finished` /
      // `transcription-failed` events.
      await api.transcribeConversation(conversation.id);
    } catch (e) {
      setRecordingError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className="workspace-page">
      <div className="workspace-content flex min-h-0 flex-1 flex-col overflow-hidden">
      <header className="workspace-header">
        <div><h1 className="workspace-title">Meetings</h1><p className="workspace-subtitle">Record conversations and turn them into searchable memory on this Mac.</p></div>
        <span className="min-w-0 flex-1" />
        {recording ? (
          <div className="flex items-center gap-2 rounded-lg border border-warning/30 bg-warning/10 px-2.5 py-1.5">
            <RadioIcon className="size-4 animate-pulse text-warning" />
            <span className="text-sm text-warning">
              Recording · {formatClock(elapsed)}
            </span>
          </div>
        ) : null}
      </header>

      <div className="flex flex-col gap-3 border-b py-5">
        {warnings.map((warning) => (
          <div
            key={warning.id}
            role="status"
            className="flex items-start gap-2 rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning"
          >
            <InfoIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
            <span className="min-w-0 flex-1">{warning.message}</span>
            <button
              type="button"
              aria-label="Dismiss warning"
              title="Dismiss"
              className="shrink-0 rounded p-0.5 hover:bg-warning/20"
              onClick={() => setWarnings((prev) => prev.filter((w) => w.id !== warning.id))}
            >
              <XIcon className="size-3.5" aria-hidden="true" />
            </button>
          </div>
        ))}
        {batchRunning || waiting.length > 0 ? (
          <div
            role="status"
            className="flex items-center gap-2 rounded-md border bg-muted/40 px-3 py-2 text-xs"
          >
            {batchRunning ? (
              <>
                <Loader2Icon className="size-3.5 animate-spin" aria-hidden="true" />
                <span className="flex-1">
                  Transcribing recording {Math.min(batch.done + 1, batch.total)} of {batch.total}…
                </span>
              </>
            ) : (
              <>
                <span className="flex-1">
                  {waiting.length === 1
                    ? "1 recording hasn't been transcribed yet."
                    : `${waiting.length} recordings haven't been transcribed yet.`}
                </span>
                <Button size="sm" variant="outline" onClick={transcribeAll}>
                  Transcribe all
                </Button>
              </>
            )}
          </div>
        ) : null}
        {recordingError ? (
          <p role="alert" className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {recordingError}
          </p>
        ) : null}
        <div className="flex items-center gap-2">
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="text-sm font-medium">
              {settings.recordingKind === "voice-note" ? "Voice note" : "Meeting recording"}
            </span>
            <span className="text-xs text-muted-foreground">
              Captures your microphone, then transcribes on this Mac. Meetings
              fold into the memory layer with action items; voice notes save as
              lighter recordings.
            </span>
          </div>
          {recording ? (
            <Button variant="destructive" onClick={stop}>
              <CheckIcon data-icon="inline-start" />
              Stop &amp; transcribe
            </Button>
          ) : (
            <Button onClick={start} disabled={starting}>
              <MicIcon data-icon="inline-start" />
              {starting ? "Starting…" : "Start recording"}
            </Button>
          )}
        </div>
        {!recording ? (
          <div className="flex gap-1 rounded-lg bg-muted/60 p-1 w-fit">
            {(
              [
                ["meeting", <MicIcon key="mic" data-icon="inline-start" className="size-3.5" />, "Meeting"],
                ["voice-note", <StickyNoteIcon key="note" data-icon="inline-start" className="size-3.5" />, "Voice note"],
              ] as const
            ).map(([kind, icon, label]) => {
              const selected = settings.recordingKind === kind;
              return (
                <button
                  key={kind}
                  type="button"
                  aria-pressed={selected}
                  onClick={() => saveSetting("recordingKind", kind)}
                  className={`inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                    selected
                      ? "border border-border bg-card text-foreground"
                      : "text-muted-foreground hover:text-foreground"
                  }`}
                >
                  {icon}
                  {label}
                </button>
              );
            })}
          </div>
        ) : null}
      </div>

      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
        {sorted.length === 0 ? (
          <Empty>
            <EmptyHeader>
              <EmptyTitle>No meetings yet</EmptyTitle>
              <EmptyDescription>
                Record a meeting below to capture and transcribe what was said.
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          sorted.map((conversation) => {
            const isBusy = transcribing.has(conversation.id);
            const hasTranscript = Boolean(conversation.transcript);
            const transcribeError = transcriptionErrors[conversation.id];
            return (
              <div
                key={conversation.id}
                className="flex flex-col gap-3 border-b px-1 py-4 transition-colors hover:bg-muted/30"
              >
                <div className="flex items-start gap-2">
                  <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium">
                        {conversation.title}
                      </span>
                      {conversation.durationMs ? (
                        <Badge variant="secondary">
                          {formatDuration(conversation.durationMs)}
                        </Badge>
                      ) : null}
                    </div>
                    <p className="text-xs text-muted-foreground">
                      {formatDate(conversation.startedAt)} at{" "}
                      {formatTime(conversation.startedAt)}
                    </p>
                  </div>
                  {!hasTranscript && !isBusy ? (
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => transcribe(conversation)}
                    >
                      Transcribe now
                    </Button>
                  ) : null}
                  {hasTranscript ? (
                    <Badge className="bg-success/10 text-success">
                      Transcribed
                    </Badge>
                  ) : null}
                  {confirmDelete === conversation.id ? (
                    <Button
                      variant="destructive"
                      size="sm"
                      onClick={() => removeRecording(conversation)}
                    >
                      Confirm delete
                    </Button>
                  ) : (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="text-muted-foreground hover:text-destructive"
                      title="Delete recording"
                      aria-label="Delete recording"
                      onClick={() => setConfirmDelete(conversation.id)}
                    >
                      <Trash2Icon className="size-4" />
                    </Button>
                  )}
                </div>
                {isBusy ? (
                  <div className="flex items-center gap-2 text-sm text-muted-foreground">
                    <Loader2Icon className="size-4 animate-spin" />
                    Transcribing locally…
                  </div>
                ) : null}
                {!isBusy && transcribeError ? (
                  <p
                    role="alert"
                    className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
                  >
                    {transcribeError}
                  </p>
                ) : null}
                {conversation.transcript ? (
                  <ScrollArea className="max-h-56">
                    <pre className="whitespace-pre-wrap break-words text-sm leading-relaxed text-foreground/80">
                      {conversation.transcript}
                    </pre>
                  </ScrollArea>
                ) : null}
              </div>
            );
          })
        )}
      </div>
      </div>
    </div>
  );
}
