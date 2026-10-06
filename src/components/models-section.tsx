import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import { ModelRow, READY_MODEL_STATE, type ModelState } from "@/components/model-row";
import { api } from "@/lib/api";
import { useMemento } from "@/lib/store";
import type { ModelDownloadProgress } from "@/lib/types";

function formatMb(bytes: number): string {
  return `${Math.max(1, Math.round(bytes / (1024 * 1024)))} MB on this Mac`;
}

/** Download / remove management for the transcription model (Settings). */
export function ModelsSection() {
  const { settings } = useMemento();
  const [transcribeModel, setTranscribeModel] = useState<ModelState>({
    status: "idle",
    progress: null,
    error: null,
  });
  const [transcribeBytes, setTranscribeBytes] = useState<number | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);

  const refreshStatus = useCallback(async () => {
    try {
      const status = await api.transcriptionModelStatus(settings.transcriptionModel);
      setTranscribeModel(
        status.ready
          ? READY_MODEL_STATE
          : { status: "idle", progress: null, error: null },
      );
      setTranscribeBytes(status.ready ? status.bytes : null);
    } catch {
      /* non-native runtime or transient error; leave as-is */
    }
  }, [settings.transcriptionModel]);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

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
      await refreshStatus();
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

  async function removeTranscriptionModel() {
    setConfirmRemove(false);
    try {
      await api.deleteTranscriptionModel(settings.transcriptionModel);
      await refreshStatus();
    } catch (e) {
      setTranscribeModel({
        status: "error",
        progress: null,
        error: e instanceof Error ? e.message : String(e),
      });
    }
  }

  return (
    <div className="space-y-2 p-4">
      <ModelRow
        title="Transcription model"
        detail={
          transcribeBytes !== null
            ? formatMb(transcribeBytes)
            : "Whisper Tiny GGML, ~75 MB"
        }
        state={transcribeModel}
        onDownload={downloadTranscriptionModel}
        onRemove={removeTranscriptionModel}
        removeArmed={confirmRemove}
        onToggleRemove={() => setConfirmRemove((armed) => !armed)}
      />
      <p className="text-xs text-muted-foreground">
        Used for meeting and voice-note transcription. The smaller search model
        downloads automatically the first time it&apos;s needed. Removed models
        download again when next used.
      </p>
    </div>
  );
}
