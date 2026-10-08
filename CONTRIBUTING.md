# Contributing to Memento

Thanks for helping. Bug reports, ideas, and PRs are all welcome.

## Setup

```sh
npm install
npm run tauri dev
```

You need macOS, Node 20+, and the Rust toolchain. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for a map of the code.

## Good first contributions

- **Capture adapters** (`src-tauri/src/adapters.rs`): add a parser for a work app
  or site. Keep the privacy rule: allowlist only, never record password managers
  or system UI.
- Improvements to action-item heuristics (`src-tauri/src/actions.rs`).
- Docs, screenshots, and bug reports with clear repro steps.

## Pull requests

- Keep PRs focused, and describe what changed and why.
- Run `npm run build` and `cargo check` in `src-tauri/` before submitting.
- Anything that sends data off the device needs a discussion first. Local-only
  is a core promise of the project.
