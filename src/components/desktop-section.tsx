import { useState } from "react";
import {
  CrosshairIcon,
  MousePointerClickIcon,
  RefreshCwIcon,
  TypeIcon,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldGroup,
  FieldLabel,
  FieldSet,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { api } from "@/lib/api";

export function DesktopSection() {
  const [info, setInfo] = useState<string>("");
  const [appName, setAppName] = useState("");
  const [text, setText] = useState("");
  const [activating, setActivating] = useState(false);
  const [typing, setTyping] = useState(false);
  const [clicking, setClicking] = useState(false);
  const [status, setStatus] = useState<string | null>(null);

  async function inspect() {
    const f = await api.focused();
    const lines = [
      f.app ?? "—",
      f.windowTitle ? `Window: ${f.windowTitle}` : null,
      f.focusedText
        ? `Focused: ${f.focusedText.slice(0, 120)}`
        : null,
      f.selectedText ? `Selected: ${f.selectedText.slice(0, 120)}` : null,
    ].filter(Boolean);
    setInfo(lines.join("\n") || "Nothing focused right now");
  }

  async function activate() {
    setActivating(true);
    try {
      const ok = await api.desktopActivate(appName.trim());
      setStatus(ok ? `Activated ${appName.trim()}` : "Could not activate");
    } finally {
      setActivating(false);
    }
  }

  async function typeIntoFocused() {
    if (!text.trim()) return;
    setTyping(true);
    try {
      const f = await api.focused();
      const pid = f.pid;
      if (pid === null) {
        setStatus("No focused app");
        return;
      }
      const handle = await api.desktopFind(
        pid,
        ["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField"],
        "",
      );
      if (handle === null) {
        setStatus("Found no text field to type into");
        return;
      }
      const ok = await api.desktopSetText(handle, text);
      setStatus(ok ? "Wrote text into the focused field" : "Write failed");
      setText("");
    } finally {
      setTyping(false);
    }
  }

  async function moveCursorToFocused() {
    setClicking(true);
    try {
      const f = await api.focused();
      const pid = f.pid;
      if (pid === null) {
        setStatus("No focused app");
        return;
      }
      const handle = await api.desktopFind(
        pid,
        ["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField", "AXButton"],
        "",
      );
      if (handle === null) {
        setStatus("Found no element to click");
        return;
      }
      const bounds = await api.desktopGetBounds(handle);
      if (!bounds) {
        setStatus("Could not read element position");
        return;
      }
      const [x, y, w, h] = bounds;
      const cx = x + w / 2;
      const cy = y + h / 2;
      await api.desktopMoveCursor(cx, cy, {
        steps: 30,
        stepDelayMs: 4,
        preClickDelayMs: 80,
        holdMs: 40,
        doClick: true,
      });
      setStatus(`Moved cursor & clicked at (${Math.round(cx)}, ${Math.round(cy)})`);
    } finally {
      setClicking(false);
    }
  }

  return (
    <FieldSet>
      <div className="flex items-center justify-between">
        <FieldLegendInline />
      </div>
      <FieldGroup>
        <Field orientation="horizontal">
          <FieldContent>
            <FieldLabel>Live focused capture</FieldLabel>
            <FieldDescription>
              The app, window, and focused/selected text Memento is reading via
              Accessibility.
            </FieldDescription>
          </FieldContent>
          <Button variant="outline" size="sm" onClick={inspect}>
            <RefreshCwIcon data-icon="inline-start" />
            Refresh
          </Button>
        </Field>
        {info ? (
          <div className="rounded-lg border bg-muted/40 p-2.5">
            <pre className="whitespace-pre-wrap break-words text-xs leading-relaxed text-muted-foreground">
              {info}
            </pre>
          </div>
        ) : null}
        <Separator />
        <Field orientation="responsive">
          <FieldContent>
            <FieldLabel>Activate an app</FieldLabel>
            <FieldDescription>
              Bring an app to the front (e.g. "Slack"). This is the entry point
              for agent-driven desktop automation.
            </FieldDescription>
          </FieldContent>
          <Input
            value={appName}
            onChange={(e) => setAppName(e.target.value)}
            placeholder="App name"
            className="max-w-44"
          />
          <Button variant="outline" size="sm" onClick={activate} disabled={activating}>
            <MousePointerClickIcon data-icon="inline-start" />
            {activating ? "…" : "Activate"}
          </Button>
        </Field>
        <Separator />
        <Field orientation="responsive">
          <FieldContent>
            <FieldLabel>Type into focused field</FieldLabel>
            <FieldDescription>
              Write text into the focused text field of the frontmost app via AX.
            </FieldDescription>
          </FieldContent>
          <Input
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="Text to type…"
            className="max-w-64"
          />
          <Button variant="outline" size="sm" onClick={typeIntoFocused} disabled={typing}>
            <TypeIcon data-icon="inline-start" />
            {typing ? "…" : "Type"}
          </Button>
        </Field>
        <Separator />
        <Field orientation="responsive">
          <FieldContent>
            <FieldLabel>Click focused element</FieldLabel>
            <FieldDescription>
              Glide the cursor to the frontmost app&apos;s focused field/button
              and click it, with a human-like stepped motion.
            </FieldDescription>
          </FieldContent>
          <Button
            variant="outline"
            size="sm"
            onClick={moveCursorToFocused}
            disabled={clicking}
          >
            <CrosshairIcon data-icon="inline-start" />
            {clicking ? "Moving…" : "Move & click"}
          </Button>
        </Field>
        {status ? (
          <p className="text-xs text-muted-foreground">{status}</p>
        ) : null}
      </FieldGroup>
    </FieldSet>
  );
}

function FieldLegendInline() {
  return (
    <div className="mb-1 text-base font-medium">
      Desktop interaction
      <span className="ml-2 hidden rounded-md bg-muted px-1.5 py-0.5 text-xs font-normal text-muted-foreground sm:inline">
        agent-ready · Accessibility
      </span>
    </div>
  );
}
