# Design QA — Capture exclusions

Reference: Minimi data-management screenshot supplied by the user.

## Visual comparison

- PASS: The information hierarchy matches the reference: section title and description, inset excluded-app list, per-app switches, and blocked-domain input/action.
- PASS: Toggle semantics are explicit and match the reference: on means excluded.
- PASS: The implementation uses the existing Memento macOS settings shell, type scale, spacing, borders, and blue accent rather than copying Minimi branding.
- PASS: The installed-app list has a bounded, independently scrollable area so a large system catalog does not make the settings page unmanageable.
- PASS: Controls have visible hover/focus behavior and accessible names.

## Functional checks

- PASS: Toggling an app updates the excluded state immediately.
- PASS: Adding a valid domain renders it in the blocked list; removal is available beside each domain.
- PASS: Invalid domains are rejected with an inline error.
- PASS: Backend matching uses stable bundle identifiers and case-insensitive comparison.
- PASS: Blocking a domain also blocks its subdomains, without matching lookalike hosts.

## Severity audit

- P0: none
- P1: none
- P2: none

Status: passed.
