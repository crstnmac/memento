# Architecture

Developer reference for Memento's internals. For the overview, see the [README](../README.md).

## Stack

- **Shell** — Tauri 2 (Rust backend in `src-tauri/`)
- **Frontend** — React + TypeScript + Vite in `src/`
- **UI** — shadcn/ui (nova preset, Base UI primitives)
- **Storage** — SQLite (rusqlite, bundled) + FTS5 in `src-tauri/src/db.rs`;
  Obsidian-compatible Markdown vault (`src-tauri/src/vault.rs`)
- **Local AI** — native **Whisper Tiny** transcription (whisper.cpp bindings
  via [`transcribe-rs`](https://crates.io/crates/transcribe-rs),
  Metal-accelerated). No embedding models — the Markdown vault is the memory.
- **OS integration** — Accessibility (AX) capture, menu-bar tray, audio capture


## The views

- **Timeline** — search and filter your captured memory; read and delete
  entries.
- **Threads** — work sessions grouped by app, with **eras of state**: a frozen-
  per-hour version history per thread (word counts, live/frozen badges).
- **Meetings** — start/stop a recording (meeting or **voice note**); it
  transcribes locally and turns the transcript into a titled meeting memory +
  action items, or a lighter voice note. micwatch can auto-start/auto-stop.
- **Actions** — commitments extracted from memory and meetings: urgent/unread
  badges, scheduled reminders, batch nudge banner, local summary agent,
  resolve/dismiss/+1h.
- **Settings** — permission status, capture pause, micwatch controls, reminder
  nudges, audio/transcription model choices, MCP server, and your database path.

## Local MCP server ("LLM brain")

Settings → **LLM brain — local MCP** starts an on-device server exposing the
memory/meeting/agent tools over **SSE on 127.0.0.1** (Model Context Protocol).
Claude Desktop connects through the bundled `mcpStdio.cjs` bridge — press
**Add to Claude Desktop** (config is written; restart Claude to pick it up).
All tool calls run against the local database; nothing is relayed.

## Command surface

All Tauri commands live in `src-tauri/src/lib.rs`; the typed JS client is
`src/lib/api.ts`. Server events (`memory-new`, `recording-stopped`,
`recording-started`, `recording-auto-stopped`, `reminders-due`,
`versions-updated`, `pause-changed`, `new-memory`, `visit`) keep the UI in
sync with capture, meeting recording, and nudges.

## Layout

- `src-tauri/src/lib.rs` — Tauri setup, tray, event wiring, ~45 commands
- `src-tauri/src/db.rs` — SQLite schema + CRUD + FTS5 keyword search +
  memory versions + reminder queue
- `src-tauri/src/monitor.rs` — background Accessibility watcher (app visits +
  visible-text capture)
- `src-tauri/src/ax.rs` — Accessibility (AX) helpers via objc2
- `src-tauri/src/audio.rs` — microphone + system-audio recording (cpal /
  ScreenCaptureKit)
- `src-tauri/src/transcribe.rs` — native transcription: GGML model download &
  cache, WAV decode/resample/mixdown, whisper.cpp inference, transcript events
- `src-tauri/src/adapters.rs` — per-app capture allowlist: one parser per
  supported app/web service, privacy gates, generic activity returns nothing
- `src-tauri/src/actions.rs` — local action-item heuristics (urgency/scheduling)
- `src-tauri/src/versions.rs` — hourly thread snapshot + freeze worker
- `src-tauri/src/reminders.rs` — due-reminder batching, DND hook, nudge worker
- `src-tauri/src/micwatch.rs` — auto start/stop + max-duration clamp
- `src-tauri/src/mcp_http.rs` — local SSE MCP server; `local_tools.rs` — tool surface
- `src-tauri/src/vault.rs` — Obsidian-compatible Markdown memory vault
  (export + `notes/` ingest, 60s sync worker)
- `src-tauri/src/tray.rs` — menu-bar tray
- `mcpStdio.cjs` — Claude Desktop ⇄ local MCP bridge
- `src/lib/store.tsx` — frontend state + event-driven transcription status
- `src/components/*` — the shadcn views

## Notes & limitations

- **Models**: the Whisper Tiny GGML model is fetched from the Hugging Face CDN on
  first use into `~/Library/Application Support/dev.poppy.memento/models/`. All
  computation is local; no memory content is ever sent to a server.
- **System audio** (other participants) is not yet wired up. The extension point
  exists (`audio::spawn_system`) but needs the Swift ScreenCaptureKit bridge,
  which is not bundled yet. Microphone capture works.
- The bundle identifier is `dev.poppy.memento`; local data lives in
  `~/Library/Application Support/dev.poppy.memento/memento.db`.
- The original Native SDK demo (`core.ts`, `app.native`) is archived in `legacy/`.
