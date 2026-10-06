import { useCallback, useEffect, useState } from "react";
import { ServerCogIcon } from "lucide-react";

import { Badge } from "@/components/ui/badge";
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
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api";
import type { McpState } from "@/lib/types";

const TOOLS = [
  { id: "search_memory", args: { query: "", limit: 5 } },
  { id: "list_active_threads", args: { limit: 10 } },
  { id: "get_latest_context", args: {} },
  { id: "meeting_memory", args: {} },
  { id: "query_agent_items", args: {} },
  { id: "conversation_timeline", args: { limit: 20 } },
  { id: "get_thread_eras", args: { threadId: 1 } },
];

export function McpSection() {
  const [query, setQuery] = useState("");
  const [running, setRunning] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mcp, setMcp] = useState<McpState | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [port, setPort] = useState("3961");
  const [installMsg, setInstallMsg] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const state = await api.mcpState();
      setMcp(state);
      setEnabled(state.running);
      setPort(String(state.port));
    } catch (e) {
      console.error(e);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function toggle(next: boolean) {
    setEnabled(next);
    try {
      const state = await api.mcpSetEnabled(next, Number(port) || undefined);
      setMcp(state);
      setEnabled(state.running);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setEnabled(mcp?.running ?? false);
    }
  }

  async function installClaude() {
    setInstallMsg(null);
    try {
      const msg = await api.mcpInstallClaudeDesktop();
      setInstallMsg(msg);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function run(toolId: string) {
    setRunning(toolId);
    setError(null);
    setResult(null);
    try {
      const base = TOOLS.find((t) => t.id === toolId)?.args ?? {};
      const args =
        toolId === "search_memory" ? { query, limit: 5 } : base;
      const out = await api.mcpCall(toolId, args);
      setResult(JSON.stringify(out, null, 2));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning(null);
    }
  }

  return (
    <FieldSet>
      <div className="mb-1 flex items-center gap-2 text-base font-medium">
        <ServerCogIcon className="size-4" />
        LLM brain — local MCP
        <span className="ml-auto hidden rounded-md bg-muted px-1.5 py-0.5 text-xs font-normal text-muted-foreground sm:inline">
          all local · no relay
        </span>
      </div>
      <FieldGroup>
        <Field orientation="responsive">
          <FieldContent>
            <FieldLabel>On-device MCP server</FieldLabel>
            <FieldDescription>
              Serves the memory/meeting/agent tools over SSE on 127.0.0.1 so an
              external LLM can read your local context — no backend round-trip.
            </FieldDescription>
          </FieldContent>
          <div className="flex items-center gap-2">
            {enabled ? (
              <Badge className="bg-success/10 text-success">Listening</Badge>
            ) : (
              <Badge variant="secondary">Stopped</Badge>
            )}
            <Switch checked={enabled} onCheckedChange={toggle} />
          </div>
        </Field>
        {mcp ? (
          <>
            <Separator />
            <Field orientation="responsive">
              <FieldContent>
                <FieldLabel>SSE endpoint</FieldLabel>
                <FieldDescription>
                  Point an MCP client here (EventSource stream). Port is
                  configurable below; pick a free loopback port.
                </FieldDescription>
              </FieldContent>
              <div className="flex items-center gap-2">
                <Input
                  aria-label="MCP port"
                  type="number"
                  className="max-w-24"
                  value={port}
                  onChange={(e) => setPort(e.target.value)}
                  onBlur={() => { if (mcp.running) toggle(false).then(() => toggle(true)); }}
                />
                <code className="truncate rounded-md bg-muted px-2 py-1 font-mono text-xs">
                  {mcp.url}
                </code>
              </div>
            </Field>
            <Separator />
            <Field orientation="responsive">
              <FieldContent>
                <FieldLabel>Claude Desktop</FieldLabel>
                <FieldDescription>
                  Add the poppy stdio bridge to Claude Desktop&apos;s MCP
                  servers (writes claude_desktop_config.json, then restart
                  Claude).
                </FieldDescription>
              </FieldContent>
              <Button
                variant="outline"
                size="sm"
                onClick={installClaude}
                disabled={!enabled}
              >
                Add to Claude Desktop
              </Button>
            </Field>
          </>
        ) : null}
        {installMsg ? (
          <p className="whitespace-pre-wrap text-xs text-muted-foreground">
            {installMsg}
          </p>
        ) : null}
        <Separator />
        <Field orientation="responsive">
          <FieldContent>
            <FieldLabel>search_memory query</FieldLabel>
            <FieldDescription>
              Try a tool below; results come from your local database. These are
              the same tools the MCP server exposes.
            </FieldDescription>
          </FieldContent>
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search query…"
            className="max-w-56"
          />
        </Field>
        <div className="flex flex-wrap gap-2">
          {TOOLS.map((tool) => (
            <Button
              key={tool.id}
              variant="outline"
              size="sm"
              onClick={() => run(tool.id)}
              disabled={running !== null}
            >
              {running === tool.id ? "Running…" : tool.id}
            </Button>
          ))}
        </div>
        {error ? (
          <p className="text-xs text-destructive">{error}</p>
        ) : null}
        {result ? (
          <div
            className="max-h-64 overflow-auto rounded-lg border bg-muted/40"
            aria-label="Tool result"
          >
            <pre className="min-w-0 whitespace-pre-wrap break-words p-3 text-xs leading-relaxed">
              {result}
            </pre>
          </div>
        ) : null}
      </FieldGroup>
    </FieldSet>
  );
}
