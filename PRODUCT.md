# Memento

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Memento is for non-technical people who want help remembering their workday without maintaining a system themselves. The first user is an individual knowledge worker using the macOS desktop app throughout the day.

## Product Purpose

Memento quietly captures local work activity, meetings, voice notes, and written notes so people can understand what they did, recover context, and follow through on commitments. Opening the app should answer “What happened today?” before asking the user to organize anything.

## Positioning

Memento builds a private, local-first memory of the workday from activity already happening on the Mac. It combines an automatic daily record with editable, portable Markdown memory instead of requiring manual journaling or hiding recall inside an opaque cloud service.

## Operating Context

The app runs continuously from the macOS menu bar. People open the main window to review today, find earlier context, add a note, inspect a meeting, or complete a follow-up. Accessibility and microphone permissions enable capture. Memory is stored locally in SQLite and mirrored into an Obsidian-compatible Markdown vault.

## Capabilities and Constraints

- Automatically captures supported active-app content while capture is enabled.
- Groups related captures into threads and retains hourly versions.
- Records and transcribes meetings and voice notes on-device.
- Search is keyword-first; the Obsidian-compatible Markdown vault is the
  readable, editable memory (no vector embeddings).
- Extracts and reminds users about open actions and commitments.
- Exports generated memories to Markdown and ingests editable vault notes.
- Offers optional local or user-configured LLM enrichment and a local MCP server.
- The default surface prioritizes today’s activity.
- Capture state, search, quick note, and unresolved follow-ups remain easy to reach.
- Technical configuration, model choices, MCP, permissions, and storage management belong in secondary settings surfaces.
- Captured content and the local database must remain on the user’s Mac unless the user explicitly configures a hosted model.

## Brand Commitments

The product name is Memento. The voice should be calm, plainspoken, and reassuring without anthropomorphizing the product or using technical language where ordinary language works.

The desktop interface should sit comfortably beside focused professional tools such as Linear: compact, dark, keyboard-oriented, and information-dense, while preserving Memento’s own chronological memory model.

## Evidence on Hand

The repository contains working activity capture, memory timelines, threads and versions, meeting recording, action extraction, local search, settings, and an Obsidian-compatible vault. Existing local memories provide real data for interface testing. No testimonials, customer claims, or performance benchmarks are available and none should be fabricated.

## Product Principles

- Begin with today, not with system structure.
- Capture quietly; ask the user to organize as little as possible.
- Show the evidence behind summaries and actions.
- Keep technical machinery out of everyday workflows.
- Make memory readable, editable, portable, and private.

## Accessibility & Inclusion

The primary workflows must be understandable without technical knowledge and operable by keyboard. Status must never rely on color alone, and controls should use explicit labels rather than icon-only discovery.
