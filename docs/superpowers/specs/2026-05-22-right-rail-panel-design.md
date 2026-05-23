# Right Rail Panel — Design

**Date:** 2026-05-22
**Status:** Design approved, awaiting implementation plan

## Background

The chat UI currently splits supplementary content across three places:

- A `ToolsPanel` rendered separately on the left side, always visible when its `showTools` flag is true.
- A `MemoryPanel` and `TasksPanel` that share a single slot via mutually-exclusive `activeSidePanel: "none" | "memory" | "tasks"` state, toggled from the left activity bar.

This produces three independent panel chromes competing for the same screen real estate, and the Tasks panel takes a permanent activity-bar slot even when no task list exists. Tools approvals are modal pop-ups; the Tools panel is purely a configuration surface (pre-approval rules + enable/disable individual tools).

## Goal

Combine Tools and Memory into a single right-side rail with tabs, and treat Tasks as a contextual slab inside that rail that auto-shows when a task list exists and auto-hides when none does. Reclaim the left activity bar for the few persistent controls that belong there.

## Design

### 1. Layout

```
┌──┬──────────────────────────┬────────────────────────┐
│  │                          │ [Tools] [Memory]   [×] │
│  │                          ├────────────────────────┤
│L │                          │                        │
│e │                          │  Active tab content    │
│f │      Chat content        │  — Tools list (filter  │
│t │                          │    + per-server rows)  │
│  │                          │  — or MemoryPanel with │
│b │                          │    its Context/Facts/  │
│a │                          │    Docs sub-tabs       │
│r │                          │                        │
│  │                          ├────────────────────────┤
│  │                          │ Tasks slab             │
│  │                          │  (only mounted when    │
│  │                          │   a task list exists)  │
└──┴──────────────────────────┴────────────────────────┘
```

When the rail is collapsed via the `[×]` button, it shrinks to a thin (~40px) strip on the right edge showing the Tools and Memory icons stacked vertically. Clicking either icon expands the rail directly to that tab. The Tasks slab is part of the rail and disappears with it when collapsed; it reappears in place when the rail is reopened (assuming a task list still exists).

### 2. Left activity bar

Trimmed to: **chat**, **settings**, **inspect-context**.

Removed: the memory toggle (`openSidePanel("memory")`) and tasks toggle (`openSidePanel("tasks")`).

### 3. State

Replace `activeSidePanel: "none" | "memory" | "tasks"` with:

```ts
type RightRailTab = "tools" | "memory";
type RightRailState = {
  collapsed: boolean;
  activeTab: RightRailTab;
};
```

Persisted to `localStorage` under a single key (e.g. `nebula.rightRail`). Defaults on first launch: `{ collapsed: false, activeTab: "tools" }`.

Tasks slab visibility is **derived state** — it renders iff the existing tasks data source reports a non-empty current list. No separate `tasksVisible` flag; no event to "show" or "hide" the slab.

### 4. Components

**New:** `src/components/RightRail.tsx` (~100 lines). Owns:
- The tab switcher header and `[×]` close button.
- The collapsed thin-strip view with stacked Tools/Memory icons.
- The vertical composition of (active tab content) + (Tasks slab, conditional).
- Persistence of `RightRailState` to localStorage.

**Unchanged internals:** `ToolsPanel.tsx`, `MemoryPanel.tsx`, `TasksPanel.tsx`. `RightRail` composes them; the inner components don't know they moved. MemoryPanel's existing Context/Facts/Docs sub-tabs are preserved as-is.

**Modified:** `src/App.tsx`:
- Drop memory/tasks click handlers from the activity bar.
- Drop the `showTools` flag and the standalone `<ToolsPanel />` render.
- Drop the `SidePanel` type and `activeSidePanel`/`setActiveSidePanel` state.
- Drop the existing `activeSidePanel` prop plumbing into `ChatInterface`.
- Render `<RightRail />` on the right edge of the chat view.

### 5. Behavior details

- **Default state on first launch:** rail open, Tools tab active.
- **Expanded rail width:** ~320px (match or take cue from existing panel widths if there's a convention in `ToolsPanel.tsx`/`MemoryPanel.tsx`). Not user-resizable in v1.
- **Reopening from collapsed strip:** clicking Tools icon → expand + select Tools; clicking Memory icon → expand + select Memory. The collapsed strip is not a generic "expand" affordance — each icon is a direct jump.
- **Tasks slab height:** fixed at ~40% of the rail's available height when present. The panel above scrolls within the remaining ~60%. No drag-to-resize in v1; flag as a follow-up if desired.
- **Tasks empty transition:** when the last task is deleted from the active task list, the slab unmounts. CSS `max-height` transition on the slab container is acceptable but not required for v1.
- **Persistence:** rail collapsed/expanded state and last-active tab survive reload.
- **Settings tab open elsewhere:** the rail remains visible when Settings is the active main tab — its content (tool configuration, memory inspection) is still relevant alongside settings. This mirrors current behavior for ToolsPanel.

### 6. Cleanup

Files to delete or simplify after the rail is wired:
- `App.tsx:131` — memory toggle button.
- `App.tsx:145` — tasks toggle button.
- `App.tsx:194` — standalone `<ToolsPanel />` render and `showTools` flag.
- `App.tsx:18` — `SidePanel` type.
- `ChatInterface.tsx` — drop the `activeSidePanel` / `onChangeSidePanel` props if they exist purely to switch between memory and tasks. (Verify before removal; the props may also serve unrelated concerns.)

### 7. Out of scope (v1)

- Drag-to-resize of the Tasks slab.
- Cross-machine sync of rail state (we use localStorage, not settings.json).
- Animated reveal/hide of the Tasks slab beyond CSS `max-height`.
- Per-conversation rail state (state is global, not per-conversation).

## Open questions

None at design time. Implementation may surface edge cases around the localStorage migration (existing users have `activeSidePanel` in component state, not persisted — so no data migration needed; the new state simply starts at its default).

## Files touched (estimate)

- `src/components/RightRail.tsx` — new, ~100 lines.
- `src/App.tsx` — modified.
- Possibly `src/components/ChatInterface.tsx` — only if `activeSidePanel` props leak in.
- No backend changes. No `settings.json` schema changes.
