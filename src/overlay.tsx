import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  GripVerticalIcon,
  Loader2Icon,
  Maximize2Icon,
  SendIcon,
  SparklesIcon,
  XIcon,
} from "lucide-react";

import { api } from "@/lib/api";
import { cn } from "@/lib/utils";

interface ChatMessage {
  role: "user" | "assistant";
  text: string;
}

type OverlayView = "pill" | "chat";

/**
 * Pointer interaction for the undecorated window. Three modes:
 * - "drag" (default): drag the window after moving >5px; `onSimpleClick`
 *   fires when the pointer went down and up without moving.
 * - "click": never drags; fires `onSimpleClick` for press+release within 5px.
 * - "immediate": starts the native drag right on press (dedicated grip).
 * Elements marked [data-no-drag] are excluded in drag/click modes (their own
 * click handlers run instead); [data-no-drag] is ignored in immediate mode
 * because the grip element itself is the drag source.
 */
function useWindowDrag(
  onSimpleClick?: () => void,
  mode: "drag" | "click" | "immediate" = "drag",
) {
  const start = useRef<{ x: number; y: number } | null>(null);
  const onPointerDown = (event: React.PointerEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    if (mode === "immediate") {
      event.stopPropagation();
      void getCurrentWindow().startDragging();
      return;
    }
    if ((event.target as HTMLElement).closest("[data-no-drag]")) return;
    start.current = { x: event.clientX, y: event.clientY };
  };
  const onPointerMove = (event: React.PointerEvent<HTMLElement>) => {
    if (mode !== "drag") return;
    const press = start.current;
    if (!press) return;
    if (Math.hypot(event.clientX - press.x, event.clientY - press.y) > 5) {
      start.current = null;
      void getCurrentWindow().startDragging();
    }
  };
  const onPointerUp = (event: React.PointerEvent<HTMLElement>) => {
    const press = start.current;
    start.current = null;
    if (!press || !onSimpleClick) return;
    if (Math.hypot(event.clientX - press.x, event.clientY - press.y) <= 5) {
      onSimpleClick();
    }
  };
  return { onPointerDown, onPointerMove, onPointerUp };
}

// Runs before first paint: the window is transparent, so the page must not
// paint the app background (body's bg-background) behind the pill/panel.
document.documentElement.dataset.overlay = "true";

/**
 * Floating "ask your day" surface — a pill that expands into a chat panel
 * grounded in the current screen and local memory (⌘⇧L, or click the pill).
 */
export default function OverlayChat() {
  const [view, setView] = useState<OverlayView>("pill");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [screenContext, setScreenContext] = useState<{
    app: string | null;
    windowTitle: string | null;
  } | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  // The Rust side is the source of truth for pill ↔ chat; sync on mount
  // (events fired before this page loaded would otherwise be missed) and
  // follow subsequent transitions.
  useEffect(() => {
    api
      .overlayView()
      .then((view) => {
        if (view !== "hidden") setView(view);
      })
      .catch(() => {});
    const unsubscribe = listen<string>("overlay-view", (event) => {
      setView(event.payload === "chat" ? "chat" : "pill");
    });
    return () => {
      void unsubscribe.then((off) => off());
    };
  }, []);

  useEffect(() => {
    if (view !== "chat") return;
    let cancelled = false;
    api
      .focused()
      .then((focused) => {
        if (cancelled) return;
        setScreenContext({ app: focused.app, windowTitle: focused.windowTitle });
      })
      .catch(() => setScreenContext(null));
    return () => {
      cancelled = true;
    };
  }, [view]);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages, busy]);

  const hide = useCallback(() => {
    void api.overlayHide();
  }, []);

  const collapse = useCallback(() => {
    void api.overlayCollapse();
  }, []);

  const ask = useCallback(async () => {
    const question = input.trim();
    if (!question || busy) return;
    setInput("");
    setMessages((prev) => [...prev, { role: "user", text: question }]);
    setBusy(true);
    try {
      const answer = await api.chatAsk(question);
      setMessages((prev) => [...prev, { role: "assistant", text: answer }]);
    } catch (error) {
      setMessages((prev) => [
        ...prev,
        {
          role: "assistant",
          text:
            error instanceof Error ? error.message : String(error),
        },
      ]);
    } finally {
      setBusy(false);
    }
  }, [busy, input]);

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void ask();
    }
    if (event.key === "Escape") {
      event.preventDefault();
      collapse();
    }
  };

  // Drag only via the dedicated grip handle; the pill body uses pointer-based
  // click detection (native `click` is unreliable on this unfocused window).
  const pillClick = useWindowDrag(() => void api.overlayExpand(), "click");
  const handleDrag = useWindowDrag(undefined, "immediate");
  const headerDrag = useWindowDrag();

  if (view === "pill") {
    return (
      <button
        type="button"
        {...pillClick}
        onKeyDown={(event) => {
          if (event.key === "Escape") hide();
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            void api.overlayExpand();
          }
        }}
        className="flex w-full cursor-pointer items-center gap-2 overflow-hidden whitespace-nowrap rounded-full border border-border/70 bg-card/90 px-3 py-2 text-xs font-medium text-foreground shadow-lg backdrop-blur-md transition-colors hover:bg-card"
      >
        <span
          aria-hidden
          title="Drag to move"
          onPointerDown={handleDrag.onPointerDown}
          className="-ml-1 shrink-0 cursor-grab rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground active:cursor-grabbing"
        >
          <GripVerticalIcon className="size-3.5" />
        </span>
        <SparklesIcon className="size-3.5 shrink-0 text-primary" aria-hidden />
        <span className="min-w-0 flex-1 truncate text-left">
          Ask Memento — what was I doing?
        </span>
        <span
          role="button"
          tabIndex={-1}
          data-no-drag
          aria-label="Expand chat"
          title="Expand chat"
          className="shrink-0 rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
          onClick={(event) => {
            event.stopPropagation();
            void api.overlayExpand();
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.stopPropagation();
              void api.overlayExpand();
            }
          }}
        >
          <Maximize2Icon className="size-3" />
        </span>
        <span
          role="button"
          tabIndex={-1}
          data-no-drag
          aria-label="Dismiss overlay"
          className="shrink-0 rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
          onClick={(event) => {
            event.stopPropagation();
            hide();
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.stopPropagation();
              hide();
            }
          }}
        >
          <XIcon className="size-3" />
        </span>
      </button>
    );
  }

  return (
    <div
      role="dialog"
      aria-label="Ask Memento"
      className="flex h-screen w-screen flex-col overflow-hidden rounded-xl border border-border/70 bg-card/95 shadow-2xl backdrop-blur-md"
    >
      <div
        {...headerDrag}
        className="flex cursor-grab items-center gap-2 border-b px-3 py-2 active:cursor-grabbing"
      >
        <GripVerticalIcon
          className="size-3.5 text-muted-foreground"
          aria-hidden
        />
        <SparklesIcon className="size-3.5 text-primary" aria-hidden />
        <span className="text-xs font-semibold">Ask your day</span>
        {screenContext?.app ? (
          <span className="truncate rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
            {screenContext.app}
            {screenContext.windowTitle ? ` · ${screenContext.windowTitle}` : ""}
          </span>
        ) : null}
        <span className="min-w-0 flex-1" />
        <button
          type="button"
          data-no-drag
          aria-label="Collapse to pill"
          className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          onClick={collapse}
        >
          <XIcon className="size-3.5" />
        </button>
      </div>

      <div ref={scrollRef} className="min-h-0 flex-1 space-y-2.5 overflow-y-auto px-3 py-3">
        {messages.length === 0 ? (
          <p className="text-xs leading-relaxed text-muted-foreground">
            Ask about your day — Memento answers from what it captured on this
            Mac: what you were doing, past context, and open follow-ups.
          </p>
        ) : null}
        {messages.map((message, index) => (
          <div
            key={index}
            className={cn(
              "max-w-[85%] whitespace-pre-wrap break-words rounded-lg px-2.5 py-1.5 text-xs leading-relaxed",
              message.role === "user"
                ? "ml-auto bg-primary/10 text-foreground"
                : "bg-muted text-foreground/90",
            )}
          >
            {message.text}
          </div>
        ))}
        {busy ? (
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <Loader2Icon className="size-3 animate-spin" aria-hidden />
            Thinking…
          </div>
        ) : null}
      </div>

      <div className="border-t p-2">
        <div className="flex items-center gap-1.5 rounded-lg border bg-background px-2 py-1.5">
          <input
            value={input}
            onChange={(event) => setInput(event.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Ask about your day…"
            aria-label="Ask about your day"
            autoFocus
            className="min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-muted-foreground"
          />
          <button
            type="button"
            aria-label="Send question"
            disabled={busy || !input.trim()}
            className="rounded-md p-1 text-primary disabled:opacity-40"
            onClick={() => void ask()}
          >
            <SendIcon className="size-3.5" />
          </button>
        </div>
      </div>
    </div>
  );
}
