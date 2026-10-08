# Privacy

Memento is built so that your data never leaves your Mac. This page says exactly
what it does. If anything here is wrong, please open an issue.

## What it captures

- Visible text from a **fixed allowlist of work apps and sites** (Mail, Outlook,
  Spark, Slack, Teams, Discord, WhatsApp, Gmail, LinkedIn), via the macOS
  Accessibility API. See `src-tauri/src/adapters.rs`.
- Audio from your microphone, **only while you record** a meeting or voice note
  (or when the optional micwatch auto-start is enabled).
- Notes you type into the app.

## What it never captures

Password managers, generic web browsing, Finder, System Settings and other macOS
system UI, and any app not on the allowlist.

## Where data is stored

- `~/Library/Application Support/dev.poppy.memento/memento.db` (SQLite)
- A Markdown vault mirrored from that database, which you can open in Obsidian

You can pause capture from the menu bar, delete any memory in the app, or delete
these files at any time.

## What leaves your Mac

- **Nothing about your memory content.** There is no account, telemetry, or cloud sync.
- The one network request is a **one-time download of the Whisper Tiny model**
  from the Hugging Face CDN on first transcription.
- The optional local MCP server listens on `127.0.0.1` only.

## Verify it yourself

The project is MIT-licensed. Audit `src-tauri/src/` or watch the app with a
network monitor such as Little Snitch.
