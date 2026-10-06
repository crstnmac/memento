---
name: Memento
description: A focused graphite workspace for ambient workday memory.
colors:
  shell: "oklch(0.125 0.008 285)"
  workspace: "oklch(0.135 0.008 285)"
  surface: "oklch(0.165 0.008 285)"
  raised-surface: "oklch(0.19 0.009 285)"
  muted-surface: "oklch(0.215 0.009 285)"
  accent-surface: "oklch(0.24 0.018 285)"
  text: "oklch(0.94 0.008 285)"
  text-strong: "oklch(0.985 0.003 285)"
  muted-text: "oklch(0.66 0.012 285)"
  rule: "oklch(0.29 0.012 285 / 72%)"
  input-rule: "oklch(0.31 0.012 285)"
  violet: "oklch(0.67 0.18 285)"
  selection: "oklch(0.5 0.16 285 / 45%)"
  green: "oklch(0.72 0.15 155)"
  amber: "oklch(0.76 0.14 75)"
  red: "oklch(0.68 0.19 25)"
  scrollbar: "oklch(0.34 0.012 285)"
  scrollbar-hover: "oklch(0.43 0.016 285)"
typography:
  display:
    fontFamily: "Geist Variable, sans-serif"
    fontSize: "24px"
    fontWeight: 600
    lineHeight: 1
    letterSpacing: "-0.025em"
  title:
    fontFamily: "Geist Variable, sans-serif"
    fontSize: "14px"
    fontWeight: 600
    lineHeight: "20px"
  body:
    fontFamily: "Geist Variable, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: "20px"
  label:
    fontFamily: "Geist Variable, sans-serif"
    fontSize: "11px"
    fontWeight: 500
    lineHeight: "16px"
rounded:
  sm: "4.6px"
  md: "6.1px"
  control: "7.7px"
  workspace: "10.8px"
  pill: "999px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "20px"
  section: "28px"
components:
  button-primary:
    backgroundColor: "{colors.violet}"
    textColor: "{colors.text-strong}"
    typography: "{typography.title}"
    rounded: "{rounded.control}"
    padding: "0 10px"
    height: "32px"
  button-outline:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    typography: "{typography.title}"
    rounded: "{rounded.control}"
    padding: "0 10px"
    height: "32px"
  input:
    backgroundColor: "transparent"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    padding: "4px 10px"
    height: "32px"
  navigation-item-active:
    backgroundColor: "{colors.accent-surface}"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "0 8px"
    height: "32px"
---

# Design System: Memento

## Overview

**Creative North Star: "The Graphite Memory Desk"**

Memento is a compact professional workspace inspired by Linear's desktop density and composure. A dim graphite shell holds one slightly brighter, bordered work surface; restrained violet marks selection, focus, and decisive action. The result feels quiet, precise, and continuously available rather than ornamental or dashboard-like.

The system serves Memento's chronological memory model. Today foregrounds evidence from the workday, Search restores context, Follow-ups expose commitments, and Settings contains technical machinery in a secondary nested rail. Linear is the craft benchmark, not a feature template.

**Key Characteristics:**

- Graphite-on-graphite hierarchy with one violet interaction voice.
- A persistent global rail surrounding a single inset workspace.
- Compact 13px controls, explicit labels, fine rules, and restrained fills.
- Route-owned scrolling inside a stable application frame.
- Brief functional motion and visible keyboard focus.

## Colors

The palette is a cool, low-chroma graphite scale. Violet is the general interaction accent; green, amber, and red remain tightly semantic.

### Primary

- **Memory Violet** (`oklch(0.67 0.18 285)`): primary actions, current states, focus rings, timeline emphasis, and active scrollbar thumbs.

### Secondary

- **Capture Green** (`oklch(0.72 0.15 155)`): active capture, paired with text.
- **Pause Amber** (`oklch(0.76 0.14 75)`): paused or cautionary state, paired with text.
- **Destructive Red** (`oklch(0.68 0.19 25)`): destructive actions, invalid fields, and errors only.

### Neutral

- **Deep Graphite Shell** (`oklch(0.125 0.008 285)`): global navigation and the darkest layer.
- **Graphite Workspace** (`oklch(0.135 0.008 285)`): outer frame around the inset page.
- **Ink Surface** (`oklch(0.165 0.008 285)`): primary page and field background.
- **Raised Graphite** (`oklch(0.19 0.009 285)`): cards, timeline source marks, and bounded groups.
- **Quiet Graphite** (`oklch(0.215 0.009 285)`): neutral hover and muted fills.
- **Soft Rule** (`oklch(0.29 0.012 285 / 72%)`): borders, separators, and structural divisions.
- **Paper Text** (`oklch(0.94 0.008 285)`): primary content.
- **Quiet Text** (`oklch(0.66 0.012 285)`): metadata, subtitles, and inactive navigation.
- **Scrollbar Graphite** (`oklch(0.34 0.012 285)`): thin thumbs; hover uses `oklch(0.43 0.016 285)`.

**The One Violet Voice Rule.** Violet means selected, focused, current, or primary action. Do not spend it on neutral decoration.

**The Labeled Status Rule.** Green, amber, and red never carry meaning alone.

## Typography

**Display Font:** Geist Variable (sans-serif fallback)  
**Body Font:** Geist Variable (sans-serif fallback)  
**Label Font:** Geist Variable (sans-serif fallback)

**Character:** One compact grotesk family keeps the product calm and operational. Hierarchy comes from modest changes in weight and size, not decorative type.

### Hierarchy

- **Display** (600, 24px, 1, `-0.025em`): primary route title.
- **Title** (600, 14px, 20px): section headings, memory titles, and emphasized rows.
- **Body** (400, 13px, 20px): default interface copy; reading passages may rise to 14px/24px and stay near 70ch.
- **Label** (500, 11px, 16px): status and metadata; the Settings rail heading uses uppercase with `0.08em` tracking.

**The Compact Hierarchy Rule.** Prefer one clear step in size or weight at a time; avoid ornamental type, excessive uppercase, and competing headlines.

## Layout

The document, root, and outer shell are fixed to the viewport and never scroll. The shell has a 760px minimum width. Its global navigation rail is 240px wide, owns internal overflow, and sits on the deepest graphite. Each route occupies one `.workspace-page`: an 8px inset on top, right, and bottom, flush to the rail on the left, with a rounded border and clipped overflow.

Inside the page, `.workspace-scroll` owns vertical movement. Standard content is centered at a 64rem maximum with 28px horizontal padding, 40px at large widths, and 48px bottom clearance. A shared 96px minimum header aligns the 24px title, 13px subtitle, and trailing actions.

Settings adds a nested 208px category rail inside the workspace, subordinate through a 20% raised-surface tint and right rule. The detail pane centers at 48rem with 32px padding, rising to 48px on large screens.

At `max-width: 900px`, both navigation contexts remain visible: the global rail becomes 208px, Settings becomes 176px, and workspace padding becomes 20px. This corrected narrow shell is required down to the 760px application minimum.

Scrolling regions use thin 8px scrollbars with transparent tracks, pill thumbs, a 2px transparent inset border, graphite rest/hover states, and violet while active. Firefox styling mirrors the quiet thumb and transparent track.

**The Stable Frame Rule.** Keep the shell fixed and assign overflow to the rail, route body, list, or detail pane that owns the content.

## Elevation & Depth

Depth is primarily tonal and structural. The global shell, page, raised controls, and dividers establish hierarchy without floating card stacks. The inset workspace alone receives a persistent broad shadow; modal layers may use stronger utility elevation.

### Shadow Vocabulary

- **Workspace Lift** (`0 14px 40px oklch(0.05 0.01 285 / 30%)`): separates the single route plane from the shell.
- **Focus Halo** (`0 0 0 3px color-mix(in oklch, var(--ring), transparent 50%)`): keyboard state feedback, not resting elevation.

**The One Lifted Plane Rule.** The route workspace is the persistent lifted plane. Ordinary rows stay flat; fills are reserved for selection, bounded settings groups, and feedback.

## Shapes

The base radius is `0.48rem` (about 7.7px). Compact controls step down to 4.6–6.1px, workspace and dialogs step up to about 10.8px, and badges, dots, switches, and scrollbar thumbs use pill geometry. Timeline source marks are 28px rounded squares; the live “Now” point is circular. Borders are one-pixel structural rules.

## Components

### Workspace and Page Header

- **Workspace:** one bordered `surface` plane, 8px inset, approximately 11px corners, clipped overflow, and Workspace Lift.
- **Header:** at least 96px tall with 24px top and 20px bottom padding, a bottom rule, title, subtitle, and aligned actions.
- **Content:** centered to 64rem generally and 48rem in settings detail.

### Buttons

- **Shape:** about 7.7px radius; default 32px, small 28px, large 36px.
- **Primary:** violet with near-white text and 10px horizontal padding.
- **Quiet variants:** outline, secondary, and ghost use a structural border or neutral tonal fill.
- **States:** neutral or 20% tint hover, visible violet three-pixel halo, one-pixel pressed translation, and 50% disabled opacity.

### Inputs and Fields

- **Style:** 32px high, transparent or subtly raised, one `input-rule` border, 10px horizontal padding.
- **Focus:** violet border and three-pixel translucent halo.
- **Error / Disabled:** red border/halo for invalid input; muted fill and 50% opacity when disabled.
- **Settings rows:** at least 64px high with 16px horizontal and 12px vertical padding, separated by rules.

### Navigation

- **Global rail:** 240px desktop / 208px at `<=900px`; 12px horizontal and 16px vertical padding.
- **Rail header:** omitted. Navigation begins immediately with Today; there is no logo, wordmark, or redundant home control.
- **Destinations:** 32px rows, 16px icons, 13px medium labels, and 8px horizontal padding. Hover uses quiet graphite; current route uses violet-tinted graphite.
- **Capture:** a 32px labeled status row; the 10px green dot breathes only while active and stops under reduced motion.

### Settings Nested Rail

- **Structure:** 208px inner rail separated from a 48rem detail pane; 176px at `<=900px`.
- **Categories:** 36px icon-plus-text rows with 16px icons and 14px labels; active treatment matches global navigation.
- **Groups:** rounded bordered containers on a 40% raised-surface tint; related rows share one group with internal dividers.

### Timeline and Lists

- **Timeline:** 64px time gutter, fine vertical rule, 28px rounded-square source marks, and open 112px-minimum entries with 20px vertical rhythm.
- **Now marker:** violet dot, hairline, and text label.
- **Rows:** open and flat with 16–20px vertical padding, clear title, quiet metadata, and subtle hover. Empty states remain plain workspace content.

### Scrollbars

- **Track:** transparent.
- **Thumb:** 8px rail, pill silhouette, graphite at rest, lighter graphite on hover, violet while active.
- **Ownership:** only the region owning overflow exposes a scrollbar.

## Do's and Don'ts

### Do:

- **Do** preserve the graphite shell, inset work surface, and violet interaction hierarchy.
- **Do** use the shared workspace, header, and content primitives for every route.
- **Do** preserve both rails at `<=900px` using 208px / 176px widths.
- **Do** use open rows, fine separators, and compact explicit controls before filled containers.
- **Do** retain visible keyboard focus and status labels.
- **Do** keep scrollbars quiet, thin, and consistent.

### Don't:

- **Don't** turn the workspace into a dashboard of floating cards.
- **Don't** reintroduce the obsolete daylight paper and cornflower palette.
- **Don't** collapse or overlay navigation at the 900px breakpoint.
- **Don't** put overflow on the document or outer shell.
- **Don't** use shadows, gradients, glass effects, or bright accents as decoration.
- **Don't** hide primary workflows or status behind icon-only controls.
