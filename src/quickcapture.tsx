import { useCallback, useEffect, useRef, useState } from "react";
import { PlusIcon, XIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";

export default function QuickCapture() {
  const [text, setText] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const save = useCallback(async () => {
    const trimmed = text.trim();
    if (!trimmed || saving) return;
    setSaving(true);
    try {
      await api.insertNote(trimmed);
      setText("");
      setSaved(true);
      setTimeout(() => setSaved(false), 1200);
    } finally {
      setSaving(false);
    }
  }, [text, saving]);

  const hide = useCallback(async () => {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().hide();
  }, []);

  // Focus the input when summoned, and wire up Escape-to-hide. Listen on the
  // capture phase so Escape is handled even when focus sits on the textarea.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    Promise.all([
      import("@tauri-apps/api/event").then(({ listen }) =>
        listen("quickcapture-focus", () => {
          window.getSelection?.()?.removeAllRanges();
          inputRef.current?.focus();
        }).then((fn) => (unlisten = fn)),
      ),
    ]);
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        hide();
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      unlisten?.();
    };
  }, [hide]);

  return (
    <div className="flex h-screen flex-col gap-2 bg-background p-2 text-foreground">
      <Textarea
        ref={inputRef}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            save();
          }
        }}
        placeholder="Capture a memory…  (⌘/Ctrl+Enter to save)"
        className="min-h-0 flex-1 resize-none"
        autoFocus
      />
      <div className="flex items-center justify-end gap-2">
        <span
          className={cn(
            "mr-auto text-xs",
            saved ? "text-success" : "text-muted-foreground",
          )}
        >
          {saved ? "Saved to your memory" : "ESC to hide"}
        </span>
        <Button variant="ghost" size="sm" onClick={hide} aria-label="Close quick capture"><XIcon /> Close</Button>
        <Button size="sm" onClick={save} disabled={!text.trim() || saving}>
          {saving ? (
            "Saving…"
          ) : (
            <>
              <PlusIcon data-icon="inline-start" />
              Save
            </>
          )}
        </Button>
      </div>
    </div>
  );
}
