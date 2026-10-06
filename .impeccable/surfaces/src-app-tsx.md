---
version: 1
slug: "src-app-tsx"
primary_target: "src/App.tsx"
related_targets: []
---

Mode: Operate. Audience: non-technical macOS knowledge workers. Primary job: understand today's activity immediately. Primary actions: search memory, add a note, review unresolved follow-ups, pause/resume capture. Direction: a daylight workday folio—one chronological daybook rather than a dashboard or database browser. The timeline is the product mechanism and the default surface. Meetings, activity, and notes share one temporal stream; technical machinery moves to secondary settings. Memorable moment: the live “now” rule advances through the page as new memories arrive.

Approved comp: `.impeccable/mocks/daybook-a.png`.

Component grammar: open document regions separated by fine rules; filled surfaces are reserved for the selected navigation item, primary action, and active state. Corners are 8–12px and controls use a single border without shadow. Type ramp: 32px page title, 18px section title, 14px entry title, 13px body and metadata. Navigation uses explicit icon-plus-text labels.

Inventory:

| Ingredient | Commitment | Medium |
| --- | --- | --- |
| Navigation | Today, Memory, Follow-ups; Settings and capture state at bottom | Semantic buttons + Lucide icons |
| Today heading | Date, plain-language capture summary, search, Add note | Semantic HTML/CSS |
| Timeline | Time gutter, fine vertical rule, heterogeneous entries, blue Now rule | Semantic list + CSS |
| Follow-ups | Persistent narrow column with direct completion controls | Semantic list + existing actions data |
| Source marks | Small consistent app/activity glyphs | Lucide icon library |
| Primary action | Solid cornflower Add note button | Existing Button module, restyled |
| Motion | New memories settle into the live timeline; capture dot breathes gently | CSS with reduced-motion fallback |
