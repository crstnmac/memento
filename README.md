<p align="center">
  <img src="assets/icon.png" alt="Memento logo: two stacked cards on a dark rounded square" width="112">
</p>

<h1 align="center">Memento</h1>

<p align="center"><b>Your Mac remembers what you promised to do.<br>100% local. No cloud. No account.</b></p>

<p align="center">
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <img alt="Platform: macOS" src="https://img.shields.io/badge/platform-macOS-lightgrey.svg">
  <img alt="Built with Tauri 2 and Rust" src="https://img.shields.io/badge/built%20with-Tauri%202%20%2B%20Rust-orange.svg">
  <a href="https://github.com/crstnmac/memento/stargazers"><img alt="GitHub stars" src="https://img.shields.io/github/stars/crstnmac/memento?style=flat"></a>
</p>

<p align="center">
  <img src="docs/screenshots/today.png" alt="Memento's Today screen: a short summary of the day, open follow-ups, and a timeline of captured activity" width="900">
</p>

Memento is a native macOS menu-bar app that quietly turns your workday (mail,
Slack, Teams, meetings, voice notes) into searchable memory and a list of
follow-ups you owe people. Ask it *"what did I promise last Tuesday?"* from any
app with **⌘⇧L**.

Everything stays on your Mac: SQLite plus a plain-Markdown, Obsidian-compatible
vault you can open, edit, and delete. The code is open, so you can audit it.

## Why Memento

|  | Cloud screen-recorders | **Memento** |
|---|---|---|
| Where your data lives | Their servers (or opaque local blobs) | A SQLite file and Markdown files on your Mac |
| What gets captured | Often everything on screen | Only an allowlist of work apps; password managers and system UI are never recorded |
| Works without an LLM | Rarely | Yes. Keyword search (FTS5) and heuristic follow-up detection need no model |
| Open and auditable | No | Yes, MIT |
| Your memory is... | Locked in the app | Plain `.md` files you own |

## Quick start

```sh
git clone https://github.com/crstnmac/memento.git
cd memento
npm install
npm run tauri dev      # development (hot reload)
npm run tauri build    # release .app
```

Requires macOS, Node 20+, and a [Rust toolchain](https://rustup.rs). On first
launch, grant **Accessibility** (required for capture) and **Microphone**
(required for meeting recording) in System Settings > Privacy & Security. The
Settings pane opens each permission pane and shows live status.

A prebuilt, signed `.dmg` and a Homebrew cask are on the [roadmap](#roadmap).

<p align="center">
  <img src="docs/screenshots/popover.png" alt="Memento's menu-bar popover: capture status, an Ask your day box, pause and record controls, a quick note box, and open follow-ups" width="340">
</p>

<p align="center"><sub>The menu-bar popover. Screenshots use sample data.</sub></p>

## Features

- **Runs from the menu bar.** Captures quietly in the background; the main
  window opens on demand. Closing the window hides the app — it keeps running.
- **Captures only meaningful work, via adapters.** A dedicated allowlist of
  adapters covers the common work surfaces — Mail, Outlook, Spark, Slack,
  Teams, Discord, WhatsApp (native + web), Gmail, LinkedIn. Random native
  apps, generic browsing, password managers, and macOS system UI (System
  Settings, Finder, …) are never recorded. Near-duplicate screens (clocks,
  scroll, cursors) are suppressed so memory stays readable.
- **Eras of state per thread.** Captures group into threads, and every thread
  keeps an append-only hourly snapshot history (`memory_versions`): content,
  word count, parent version, frozen once the hour rolls over — so you can
  reconstruct "what was on screen at 3pm".
- **Finds action items** locally — commitments, follow-ups, and deadlines are
  extracted with heuristics (no LLM needed), tagged urgent/unread, and given a
  scheduled `remind_at` ("Send it by Friday" → Friday 9am).
- **Reminder nudge, batched.** A 60s worker batches all due reminders into ONE
  notification (missed-reminder catch-up included) and honors macOS Do Not
  Disturb / Focus before surfacing it.
- **Your memory is a Markdown vault.** Every memory is mirrored as plain
  `.md` into an Obsidian-compatible vault (activity, meetings, daily
  `[[wiki-link]]` indexes, and a `notes/` folder that is ingested back).
  Search is keyword-first (SQLite FTS5) — no opaque embeddings; you can open,
  edit, and own every memory file.
- **Meeting & voice-note pipeline.** mic (+ optional system audio) recording →
  native Whisper transcription (whisper.cpp via `transcribe-rs`, Whisper Tiny
  GGML) → titled meeting memory (deduplicated titles) or a lighter voice note.
  Transcription runs on the Rust side, so it keeps working when the window is
  closed. "micwatch" can auto-start when a meeting app comes forward and
  auto-stop when it ends, with a max-duration clamp so recordings never run
  forever.
- **Ask your day, over anything.** Open it with ⌘⇧L from any app, from
  **Ask your day** in the sidebar, or from the menu-bar popover. A floating,
  always-on-top panel answers from the current screen (frontmost app, window
  title, selected text), your captured memory, and open follow-ups. Questions
  can name a time ("what did I promise last Tuesday?"). With a local LLM it
  writes an answer; **without one it still works** and shows the matching
  memories and follow-ups it found. Everything stays on-device.
- **Timed pause & hourly check-in.** The tray can pause capture for 5/10/30/60
  minutes with auto-resume, and a quiet hourly notification summarizes the
  past hour (memories, follow-ups) honoring Do Not Disturb.
- **Local MCP server.** A built-in SSE server on `127.0.0.1` exposes the same
  memory/meeting/agent tools to external LLMs over the Model Context Protocol,
  fully on-device (`mcpStdio.cjs` bridges Claude Desktop to it).
- **Stores everything locally** on your Mac: SQLite (memories, versions, app
  visits, conversations, action items, agent runs) mirrored into the Markdown
  memory vault.

## Local MCP server ("LLM brain")

Settings → **LLM brain — local MCP** starts an on-device server exposing the
memory/meeting/agent tools over **SSE on 127.0.0.1** (Model Context Protocol).
Claude Desktop connects through the bundled `mcpStdio.cjs` bridge — press
**Add to Claude Desktop** (config is written; restart Claude to pick it up).
All tool calls run against the local database; nothing is relayed.

## Privacy

Read [PRIVACY.md](PRIVACY.md) for exactly what is captured, where it is stored,
and what never leaves your Mac.

## Roadmap

- [ ] Signed and notarized `.dmg` in GitHub Releases
- [ ] Homebrew cask (`brew install --cask memento`)
- [ ] System-audio capture for meetings (ScreenCaptureKit bridge)
- [ ] More capture adapters (see [good first issues](https://github.com/crstnmac/memento/labels/good%20first%20issue))
- [ ] Encrypted-at-rest database option

## Docs

- [Architecture and file layout](docs/ARCHITECTURE.md)
- [Contributing](CONTRIBUTING.md)

## Contributing

Issues and PRs are welcome. New capture adapters are the easiest way in; start
with [CONTRIBUTING.md](CONTRIBUTING.md). If Memento is useful to you, a star
helps others find it.

## License

[MIT](LICENSE) © 2026 Criston Mascarenhas.
