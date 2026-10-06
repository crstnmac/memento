import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  BrainCircuitIcon,
  CheckIcon,
  KeyRoundIcon,
  MicIcon,
  MonitorSmartphoneIcon,
  XIcon,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import {
  IDLE_MODEL_STATE,
  ModelRow,
  READY_MODEL_STATE,
  type ModelState,
} from "@/components/model-row";
import { api } from "@/lib/api";
import { useMemento } from "@/lib/store";
import type { ModelDownloadProgress, Permissions, PermState } from "@/lib/types";
import { cn } from "@/lib/utils";

type StepKind = "accessibility" | "microphone" | "screenRecording";

interface OnboardingProps {
  onDone: () => void;
}

const STEP_META: { kind: StepKind; title: string; desc: string; icon: typeof KeyRoundIcon; required?: boolean }[] = [
  {
    kind: "accessibility",
    title: "Accessibility",
    desc: "Required so Memento can read the active app, focused text, and window — this is how memories get captured. We never read passwords or credential apps.",
    icon: KeyRoundIcon,
    required: true,
  },
  {
    kind: "microphone",
    title: "Microphone",
    desc: "Optional — lets Memento transcribe your meetings locally. Used only while you record.",
    icon: MicIcon,
  },
  {
    kind: "screenRecording",
    title: "Screen & system audio",
    desc: "Optional — captures the other people on a call. Reopen Memento after allowing it.",
    icon: MonitorSmartphoneIcon,
  },
];

const STEP_TO_KIND: Record<string, string> = {
  accessibility: "accessibility",
  microphone: "microphone",
  screenRecording: "screen-recording",
};

export function Onboarding({ onDone }: OnboardingProps) {
  const { saveSetting, settings } = useMemento();
  const [perms, setPerms] = useState<Permissions | null>(null);
  const [skipConfirmed, setSkipConfirmed] = useState(false);
  const [completing, setCompleting] = useState(false);
  const [requested, setRequested] = useState<string | null>(null);
  const [transcribeModel, setTranscribeModel] = useState<ModelState>(IDLE_MODEL_STATE);
  const timer = useRef<number | null>(null);

  const refresh = useCallback(async () => {
    try {
      setPerms(await api.getPermissions());
    } catch {
      /* not fatal */
    }
  }, []);

  // Poll permissions every 900ms while onboarding is open.
  useEffect(() => {
    refresh();
    timer.current = window.setInterval(refresh, 900);
    return () => {
      if (timer.current) window.clearInterval(timer.current);
    };
  }, [refresh]);

  // Reflect a transcription model that is already on this Mac.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const status = await api.transcriptionModelStatus(settings.transcriptionModel);
        if (!cancelled && status.ready) setTranscribeModel(READY_MODEL_STATE);
      } catch {
        /* not fatal */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [settings.transcriptionModel]);

  useEffect(() => {
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      onDone();
    };
    document.addEventListener("keydown", dismissOnEscape, true);
    return () => document.removeEventListener("keydown", dismissOnEscape, true);
  }, [onDone]);

  const state = (kind: StepKind): PermState => perms?.[kind] ?? "notDetermined";
  const granted = (kind: StepKind) => state(kind) === "granted";
  const accessibilityGranted = granted("accessibility");
  const canFinish = accessibilityGranted || skipConfirmed;

  async function request(kind: StepKind) {
    setRequested(STEP_TO_KIND[kind]);
    // Opens the relevant System Settings pane / prompts via the
    // tauri-plugin-macos-permissions plugin.
    try {
      await api.requestPermission(STEP_TO_KIND[kind]);
    } finally {
      refresh();
    }
  }

  async function finish() {
    if (!canFinish) return;
    setCompleting(true);
    await saveSetting("onboardingComplete", "true");
    onDone();
  }

  // Downloads the Whisper Tiny GGML model natively; used by meeting and
  // voice-note transcription.
  async function downloadTranscriptionModel() {
    setTranscribeModel({ status: "busy", progress: 0, error: null });
    const unlisten = await listen<ModelDownloadProgress>(
      "model-download-progress",
      (event) => {
        const { received, total } = event.payload;
        if (total) {
          setTranscribeModel((s) =>
            s.status === "busy"
              ? { ...s, progress: Math.round((received / total) * 100) }
              : s,
          );
        }
      },
    );
    try {
      await api.prepareTranscriptionModel(settings.transcriptionModel);
      setTranscribeModel({ status: "ready", progress: 100, error: null });
    } catch (e) {
      setTranscribeModel({
        status: "error",
        progress: null,
        error: e instanceof Error ? e.message : String(e),
      });
    } finally {
      unlisten();
    }
  }

  const transcriptionModelLabel =
    settings.transcriptionModel === "whisper-tiny"
      ? "Whisper Tiny (multilingual)"
      : "Whisper Tiny (English)";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center overflow-y-auto bg-background/95 p-4 backdrop-blur-sm">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="onboarding-title"
        data-onboarding
        className="relative my-auto flex max-h-full w-full max-w-md flex-col overflow-y-auto rounded-2xl border bg-card p-6 shadow-lg"
      >
        <Button variant="ghost" size="icon-sm" className="absolute right-3 top-3" aria-label="Close setup" onClick={onDone}><XIcon /></Button>
        <div className="flex flex-col gap-1">
          <h1 id="onboarding-title" className="font-heading text-xl font-semibold tracking-tight">
            Welcome to Memento
          </h1>
          <p className="text-sm text-muted-foreground">
            A private, local memory layer for your workday. Everything stays on
            this Mac — no accounts, no cloud database.
          </p>
        </div>

        <Separator className="my-4" />

        <p className="mb-2 text-xs text-muted-foreground">
          Allow each permission below in the System Settings pane that opens.
        </p>

        <div className="flex flex-col gap-3">
          {STEP_META.map((step) => {
            const Icon = step.icon;
            const s = state(step.kind);
            const isGranted = s === "granted";
            const blocked = s === "denied" || s === "restricted";
            const isRequested = STEP_TO_KIND[step.kind] === requested && !isGranted;
            return (
              <div
                key={step.kind}
                className={cn(
                  "rounded-lg border p-3 transition-colors",
                  isGranted && "border-success/30 bg-success/5",
                )}
              >
                <div className="flex items-start gap-2.5">
                  <div className="mt-0.5 inline-flex size-7 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
                    <Icon className="size-4" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium">{step.title}</span>
                      {step.required ? (
                        <Badge variant="outline">required</Badge>
                      ) : null}
                      <span className="min-w-0 flex-1" />
                      {isGranted ? (
                        <Badge className="bg-success/10 text-success">
                          Granted
                        </Badge>
                      ) : blocked ? (
                        <Badge className="bg-destructive/10 text-destructive">
                          Denied
                        </Badge>
                      ) : isRequested ? (
                        <Badge className="bg-warning/10 text-warning">
                          Allowed in System Settings?
                        </Badge>
                      ) : (
                        <Badge variant="outline">Pending</Badge>
                      )}
                    </div>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {step.desc}
                    </p>
                    {!isGranted ? (
                      <div className="mt-2 flex items-center gap-2">
                        <Button size="sm" onClick={() => request(step.kind)}>
                          Allow {step.title}
                        </Button>
                        <Button variant="ghost" size="sm" onClick={refresh}>
                          Check again
                        </Button>
                      </div>
                    ) : null}
                  </div>
                </div>
              </div>
            );
          })}
        </div>

        <Separator className="my-4" />

        <div className="rounded-lg border p-3">
          <div className="flex items-start gap-2.5">
            <div className="mt-0.5 inline-flex size-7 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
              <BrainCircuitIcon className="size-4" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="text-sm font-medium">Local AI models</span>
                <Badge variant="outline">optional now</Badge>
              </div>
              <p className="mt-1 text-xs text-muted-foreground">
                Downloaded once and stored on this Mac — never in the cloud.
                Used for meeting and voice-note transcription.
              </p>
              <div className="mt-2.5 flex flex-col gap-2">
                <ModelRow
                  title="Transcription model"
                  detail={`${transcriptionModelLabel}, ~75 MB`}
                  state={transcribeModel}
                  onDownload={downloadTranscriptionModel}
                />
              </div>
              <p className="mt-2 text-xs text-muted-foreground">
                Skip now if you prefer — the model downloads automatically the
                first time you transcribe. Searching your memory needs no
                model.
              </p>
            </div>
          </div>
        </div>

        {!accessibilityGranted ? (
          <label className="mt-4 flex cursor-pointer items-start gap-2 text-sm">
            <input
              type="checkbox"
              checked={skipConfirmed}
              onChange={(e) => setSkipConfirmed(e.target.checked)}
              className="mt-0.5 size-4 accent-primary"
            />
            <span className="text-muted-foreground">
              I understand Memento won&apos;t capture desktop memories without
              Accessibility, and I want to start anyway.
            </span>
          </label>
        ) : null}

        <p className="mt-4 text-xs leading-5 text-muted-foreground">
          <span className="font-medium text-foreground">Tip:</span> press{" "}
          <kbd className="rounded border bg-muted px-1 py-0.5 font-sans text-[11px] text-foreground">⌘⇧L</kbd>{" "}
          anywhere to ask about your day, and{" "}
          <kbd className="rounded border bg-muted px-1 py-0.5 font-sans text-[11px] text-foreground">⌘⇧M</kbd>{" "}
          to jot a note.
        </p>

        <Separator className="my-4" />

        <div className="flex items-center justify-end gap-2">
          <span className="mr-auto flex items-center gap-1.5 text-xs text-muted-foreground">
            {accessibilityGranted ? (
              <>
                <CheckIcon className="size-3.5 text-success" />
                Ready to capture
              </>
            ) : null}
          </span>
          <Button onClick={finish} disabled={!canFinish || completing}>
            {completing
              ? "Starting…"
              : accessibilityGranted
                ? "Start using Memento"
                : "Skip for now"}
          </Button>
        </div>
      </div>
    </div>
  );
}
