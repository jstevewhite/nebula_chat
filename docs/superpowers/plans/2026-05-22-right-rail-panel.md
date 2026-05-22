# Right Rail Panel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Combine Tools and Memory into a single right-side rail with tabs, and add a Tasks slab at the bottom that auto-shows when a task list exists. Trim the left activity bar to chat / settings / inspect-context.

**Architecture:** New `RightRail.tsx` owns the tab switcher, close button, collapsed thin strip, and the conditional Tasks slab. It composes the existing `ToolsPanel.tsx`, `MemoryPanel.tsx`, and `TasksPanel.tsx` without modifying their internals. State (`collapsed`, `activeTab`) persists to `localStorage`. The right rail lives at the root layout level so it stays visible whether the active main view is Chat or Settings.

**Tech Stack:** React 19, TypeScript, Tailwind, Tauri (no backend changes).

**Spec:** `docs/superpowers/specs/2026-05-22-right-rail-panel-design.md`

**Testing note:** This project has no frontend test suite (per `CLAUDE.md`: "Frontend: No test suite currently. Manual testing via `npm run tauri dev`."). Tasks below use **manual verification** with concrete steps and expected outcomes — equivalent rigor, just no automated assertions. `npx tsc --noEmit` runs at every step.

---

## File Structure

**New file:**

- `src/components/RightRail.tsx` — top-level rail component. Owns:
  - `RightRailState` (`{ collapsed, activeTab }`) + localStorage persistence.
  - Tab header with `[Tools] [Memory]` buttons and `[×]` close button.
  - Collapsed thin-strip render with stacked Tools/Memory icons.
  - Composition of `<ToolsPanel />` or `<MemoryPanel />` based on `activeTab`.
  - Subscription to `tasks-updated` event and initial fetch of `get_conversation_tasks`, to decide whether to render the Tasks slab.
  - Conditional `<TasksPanel />` render at the bottom when `tasks.length > 0`.

**Modified files:**

- `src/App.tsx` — remove memory/tasks/tools activity-bar buttons; remove `showTools`, `activeSidePanel`, `setActiveSidePanel` state; remove `SidePanel` type export; render `<RightRail />` at root layout level.
- `src/components/ChatInterface.tsx` — remove `activeSidePanel`/`onChangeSidePanel` props from the interface and call site; remove the inline `<MemoryPanel />` / `<TasksPanel />` renders.

**Unchanged:** `ToolsPanel.tsx`, `MemoryPanel.tsx`, `TasksPanel.tsx` — these are composed by RightRail but their internals don't move.

---

## Task 1: Skeleton RightRail with Tools tab only, replace old ToolsPanel render

**Goal:** Get a basic right-side rail mounted, showing Tools content (the current `ToolsPanel`). No tab switching, no close button, no Tasks slab yet. This is a "make the new file exist and render at the root" step.

**Files:**
- Create: `src/components/RightRail.tsx`
- Modify: `src/App.tsx`

- [ ] **Step 1: Create RightRail.tsx with single Tools tab**

Write `src/components/RightRail.tsx`:

```tsx
import ToolsPanel from "./ToolsPanel";

export default function RightRail() {
    return (
        <div className="w-80 h-full border-l border-[var(--color-border-primary)] bg-[var(--color-bg-secondary)] flex flex-col shrink-0">
            {/* Header — will gain tabs + close button in Task 2-3 */}
            <div className="px-4 py-3 border-b border-[var(--color-border-primary)] text-sm font-semibold text-[var(--color-text-primary)]">
                Tools
            </div>
            <div className="flex-1 overflow-hidden">
                <ToolsPanel />
            </div>
        </div>
    );
}
```

- [ ] **Step 2: Render RightRail at the root layout in App.tsx; remove the old `{showTools && <ToolsPanel />}` line**

Edit `src/App.tsx`:

Add import near the top:

```tsx
import RightRail from "./components/RightRail";
```

Find this block at the bottom of the chat-view div (around line 194):

```tsx
          {showTools && <ToolsPanel />}
        </div>
```

Replace with:

```tsx
        </div>
```

Then, after the settings-view div closes (around line 199), add the RightRail render *outside* both tab divs but still inside the outer content `<div className="flex-1 ...">`. The structure becomes:

```tsx
      <div className="flex-1 overflow-hidden relative flex">
        <div className={activeTab === "chat" ? "flex flex-1 overflow-hidden" : "hidden"}>
          {/* ... existing chat view ... */}
        </div>

        <div className={activeTab === "settings" ? "flex flex-1 overflow-auto justify-center" : "hidden"}>
          <SettingsPage />
        </div>

        <RightRail />
      </div>
```

Also remove the `import ToolsPanel from "./components/ToolsPanel";` line at the top of App.tsx — it's no longer used directly here.

- [ ] **Step 3: Type-check**

Run: `npx tsc --noEmit`
Expected: clean (no errors).

- [ ] **Step 4: Run the app and verify visually**

Run: `npm run tauri dev`

Verify in the running app:
- A right-side panel is visible showing the Tools list (servers like FILESYSTEM, TMUX, etc.).
- The panel is visible on the Chat tab AND on the Settings tab (this is new behavior — previously Tools only showed on Chat).
- Memory and Tasks toggles on the left activity bar still work (we haven't removed them yet — they still render via the old `activeSidePanel` plumbing inside ChatInterface).
- No console errors.

- [ ] **Step 5: Commit**

```bash
git add src/components/RightRail.tsx src/App.tsx
git commit -m "feat(rail): skeleton right rail rendering ToolsPanel"
```

---

## Task 2: Add Memory tab and switching

**Goal:** Two-tab header. Click Tools → ToolsPanel. Click Memory → MemoryPanel. State lives in RightRail; not yet persisted.

**Files:**
- Modify: `src/components/RightRail.tsx`
- Modify: `src/App.tsx` (pass `recentMemories` prop down)

- [ ] **Step 1: Add tab state, header buttons, and conditional rendering to RightRail**

Replace the entire contents of `src/components/RightRail.tsx` with:

```tsx
import { useState } from "react";
import { Wrench, Brain } from "lucide-react";
import ToolsPanel from "./ToolsPanel";
import MemoryPanel from "./MemoryPanel";

type RightRailTab = "tools" | "memory";

interface RightRailProps {
    recentMemories: string[];
}

export default function RightRail({ recentMemories }: RightRailProps) {
    const [activeTab, setActiveTab] = useState<RightRailTab>("tools");

    return (
        <div className="w-80 h-full border-l border-[var(--color-border-primary)] bg-[var(--color-bg-secondary)] flex flex-col shrink-0">
            <div className="flex border-b border-[var(--color-border-primary)]">
                <TabButton
                    label="Tools"
                    icon={<Wrench size={14} />}
                    active={activeTab === "tools"}
                    onClick={() => setActiveTab("tools")}
                />
                <TabButton
                    label="Memory"
                    icon={<Brain size={14} />}
                    active={activeTab === "memory"}
                    onClick={() => setActiveTab("memory")}
                />
            </div>
            <div className="flex-1 overflow-hidden">
                {activeTab === "tools" && <ToolsPanel />}
                {activeTab === "memory" && (
                    <MemoryPanel
                        memories={recentMemories}
                        onClose={() => setActiveTab("tools")}
                    />
                )}
            </div>
        </div>
    );
}

interface TabButtonProps {
    label: string;
    icon: React.ReactNode;
    active: boolean;
    onClick: () => void;
}

function TabButton({ label, icon, active, onClick }: TabButtonProps) {
    return (
        <button
            onClick={onClick}
            className={`flex-1 flex items-center justify-center gap-1.5 px-3 py-2.5 text-xs font-semibold transition-colors ${
                active
                    ? "bg-[var(--color-bg-primary)] text-[var(--color-text-primary)] border-b-2 border-[var(--color-accent-primary)]"
                    : "text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-tertiary)]"
            }`}
        >
            {icon}
            {label}
        </button>
    );
}
```

Note: `MemoryPanel`'s `onClose` is kept satisfied by routing it to "go back to the Tools tab." We don't add a separate dismiss path because the rail itself has its own close button (added in Task 3).

- [ ] **Step 2: Pass `recentMemories` down from App.tsx**

In `src/App.tsx`, find the `<RightRail />` render and update it to pass the existing `recentMemories` state:

```tsx
        <RightRail recentMemories={recentMemories} />
```

- [ ] **Step 3: Type-check**

Run: `npx tsc --noEmit`
Expected: clean.

- [ ] **Step 4: Run the app and verify**

Run: `npm run tauri dev`

Verify:
- Right rail shows two tab buttons: Tools (with wrench icon) and Memory (with brain icon).
- Tools is active by default; Tools content is visible.
- Click Memory tab → MemoryPanel content appears with its own internal Context/Facts/Docs sub-tabs.
- Click Tools tab → Tools content reappears.
- Active tab has visible underline accent.

- [ ] **Step 5: Commit**

```bash
git add src/components/RightRail.tsx src/App.tsx
git commit -m "feat(rail): add Memory tab and tab switching"
```

---

## Task 3: Close button + collapsed thin strip

**Goal:** A `[×]` button in the tab header collapses the rail to a thin (~40px) strip showing just the Tools and Memory icons stacked vertically. Clicking an icon expands the rail and switches directly to that tab.

**Files:**
- Modify: `src/components/RightRail.tsx`

- [ ] **Step 1: Add `collapsed` state, close button, and collapsed-view render**

Replace `src/components/RightRail.tsx` with:

```tsx
import { useState } from "react";
import { Wrench, Brain, X } from "lucide-react";
import ToolsPanel from "./ToolsPanel";
import MemoryPanel from "./MemoryPanel";

type RightRailTab = "tools" | "memory";

interface RightRailProps {
    recentMemories: string[];
}

export default function RightRail({ recentMemories }: RightRailProps) {
    const [activeTab, setActiveTab] = useState<RightRailTab>("tools");
    const [collapsed, setCollapsed] = useState(false);

    if (collapsed) {
        return (
            <div className="w-10 h-full border-l border-[var(--color-border-primary)] bg-[var(--color-bg-tertiary)] flex flex-col items-center py-3 gap-2 shrink-0">
                <CollapsedIcon
                    icon={<Wrench size={18} />}
                    title="Tools"
                    onClick={() => {
                        setActiveTab("tools");
                        setCollapsed(false);
                    }}
                />
                <CollapsedIcon
                    icon={<Brain size={18} />}
                    title="Memory"
                    onClick={() => {
                        setActiveTab("memory");
                        setCollapsed(false);
                    }}
                />
            </div>
        );
    }

    return (
        <div className="w-80 h-full border-l border-[var(--color-border-primary)] bg-[var(--color-bg-secondary)] flex flex-col shrink-0">
            <div className="flex border-b border-[var(--color-border-primary)] items-stretch">
                <TabButton
                    label="Tools"
                    icon={<Wrench size={14} />}
                    active={activeTab === "tools"}
                    onClick={() => setActiveTab("tools")}
                />
                <TabButton
                    label="Memory"
                    icon={<Brain size={14} />}
                    active={activeTab === "memory"}
                    onClick={() => setActiveTab("memory")}
                />
                <button
                    onClick={() => setCollapsed(true)}
                    className="px-3 text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-tertiary)] transition-colors"
                    title="Collapse panel"
                    aria-label="Collapse right rail"
                >
                    <X size={16} />
                </button>
            </div>
            <div className="flex-1 overflow-hidden">
                {activeTab === "tools" && <ToolsPanel />}
                {activeTab === "memory" && (
                    <MemoryPanel
                        memories={recentMemories}
                        onClose={() => setActiveTab("tools")}
                    />
                )}
            </div>
        </div>
    );
}

interface TabButtonProps {
    label: string;
    icon: React.ReactNode;
    active: boolean;
    onClick: () => void;
}

function TabButton({ label, icon, active, onClick }: TabButtonProps) {
    return (
        <button
            onClick={onClick}
            className={`flex-1 flex items-center justify-center gap-1.5 px-3 py-2.5 text-xs font-semibold transition-colors ${
                active
                    ? "bg-[var(--color-bg-primary)] text-[var(--color-text-primary)] border-b-2 border-[var(--color-accent-primary)]"
                    : "text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-tertiary)]"
            }`}
        >
            {icon}
            {label}
        </button>
    );
}

interface CollapsedIconProps {
    icon: React.ReactNode;
    title: string;
    onClick: () => void;
}

function CollapsedIcon({ icon, title, onClick }: CollapsedIconProps) {
    return (
        <button
            onClick={onClick}
            title={title}
            aria-label={`Open ${title}`}
            className="p-2 rounded-lg text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-secondary)] hover:text-[var(--color-text-primary)] transition-colors"
        >
            {icon}
        </button>
    );
}
```

- [ ] **Step 2: Type-check**

Run: `npx tsc --noEmit`
Expected: clean.

- [ ] **Step 3: Run the app and verify**

Run: `npm run tauri dev`

Verify:
- `[×]` button visible on the right end of the tab header row.
- Click `[×]` → rail collapses to a thin (~40px) vertical strip with Wrench and Brain icons stacked.
- Click Wrench → rail expands, Tools tab active.
- Collapse again, click Brain → rail expands, Memory tab active.
- Chat area gets wider when the rail is collapsed.

- [ ] **Step 4: Commit**

```bash
git add src/components/RightRail.tsx
git commit -m "feat(rail): close button and collapsed thin strip"
```

---

## Task 4: Persist RightRailState to localStorage

**Goal:** The collapsed/expanded state and last-active tab survive page reload.

**Files:**
- Modify: `src/components/RightRail.tsx`

- [ ] **Step 1: Add localStorage load/save**

In `src/components/RightRail.tsx`, replace the two `useState` initializers and add a save effect.

Find:

```tsx
    const [activeTab, setActiveTab] = useState<RightRailTab>("tools");
    const [collapsed, setCollapsed] = useState(false);
```

Replace with:

```tsx
    const [activeTab, setActiveTab] = useState<RightRailTab>(() => loadState().activeTab);
    const [collapsed, setCollapsed] = useState<boolean>(() => loadState().collapsed);

    useEffect(() => {
        saveState({ activeTab, collapsed });
    }, [activeTab, collapsed]);
```

Add `useEffect` to the existing `import { useState }` line:

```tsx
import { useState, useEffect } from "react";
```

At the bottom of the file (after the `CollapsedIcon` function), add:

```tsx
const STORAGE_KEY = "nebula.rightRail";

interface RightRailState {
    collapsed: boolean;
    activeTab: RightRailTab;
}

const DEFAULT_STATE: RightRailState = {
    collapsed: false,
    activeTab: "tools",
};

function loadState(): RightRailState {
    try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (!raw) return DEFAULT_STATE;
        const parsed = JSON.parse(raw) as Partial<RightRailState>;
        return {
            collapsed: typeof parsed.collapsed === "boolean" ? parsed.collapsed : DEFAULT_STATE.collapsed,
            activeTab: parsed.activeTab === "memory" ? "memory" : "tools",
        };
    } catch {
        return DEFAULT_STATE;
    }
}

function saveState(state: RightRailState): void {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    } catch {
        // ignore: storage may be full or disabled
    }
}
```

- [ ] **Step 2: Type-check**

Run: `npx tsc --noEmit`
Expected: clean.

- [ ] **Step 3: Run the app and verify persistence**

Run: `npm run tauri dev`

Verify:
- Open Memory tab, then close the app/reload (Cmd-R inside the Tauri window if dev mode supports it; otherwise quit and relaunch).
- On relaunch: Memory tab is active.
- Collapse the rail, reload → relaunches collapsed.
- Expand + switch to Tools, reload → relaunches expanded on Tools.

- [ ] **Step 4: Commit**

```bash
git add src/components/RightRail.tsx
git commit -m "feat(rail): persist collapsed and active-tab state to localStorage"
```

---

## Task 5: Tasks slab at bottom of rail

**Goal:** When a task list exists for the active conversation, a Tasks slab appears at the bottom of the expanded rail, taking ~40% of the rail height. The tab content above scrolls within the remaining space. When the last task is deleted, the slab unmounts.

**Files:**
- Modify: `src/components/RightRail.tsx`
- Modify: `src/App.tsx` (pass `conversationId` prop)

- [ ] **Step 1: Pass conversation id from App down to RightRail**

In `src/App.tsx`, update the `<RightRail />` render:

```tsx
        <RightRail recentMemories={recentMemories} conversationId={activeConvId} />
```

- [ ] **Step 2: Add task subscription and conditional Tasks slab in RightRail**

In `src/components/RightRail.tsx`:

Update the imports at the top:

```tsx
import { useState, useEffect } from "react";
import { Wrench, Brain, X } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import ToolsPanel from "./ToolsPanel";
import MemoryPanel from "./MemoryPanel";
import TasksPanel from "./TasksPanel";
```

Update the props interface:

```tsx
interface RightRailProps {
    recentMemories: string[];
    conversationId: string | null;
}
```

Update the function signature:

```tsx
export default function RightRail({ recentMemories, conversationId }: RightRailProps) {
```

After the existing `useEffect` (the one that calls `saveState`), add task-count tracking. Note we only need the count to decide visibility; TasksPanel manages its own data on render:

```tsx
    const [taskCount, setTaskCount] = useState(0);

    useEffect(() => {
        if (!conversationId) {
            setTaskCount(0);
            return;
        }
        let cancelled = false;
        invoke<Array<unknown>>("get_conversation_tasks", { conversationId })
            .then((rows) => {
                if (!cancelled) setTaskCount(rows.length);
            })
            .catch((e) => console.error("RightRail: get_conversation_tasks failed", e));
        return () => {
            cancelled = true;
        };
    }, [conversationId]);

    useEffect(() => {
        const unlistenPromise = listen<{ conversation_id: string; tasks: Array<unknown> }>(
            "tasks-updated",
            (event) => {
                if (event.payload.conversation_id === conversationId) {
                    setTaskCount(event.payload.tasks.length);
                }
            }
        );
        return () => {
            unlistenPromise.then((unlisten) => unlisten());
        };
    }, [conversationId]);
```

Update the expanded-rail JSX so the tab content + tasks slab share vertical space:

```tsx
    return (
        <div className="w-80 h-full border-l border-[var(--color-border-primary)] bg-[var(--color-bg-secondary)] flex flex-col shrink-0">
            <div className="flex border-b border-[var(--color-border-primary)] items-stretch">
                <TabButton
                    label="Tools"
                    icon={<Wrench size={14} />}
                    active={activeTab === "tools"}
                    onClick={() => setActiveTab("tools")}
                />
                <TabButton
                    label="Memory"
                    icon={<Brain size={14} />}
                    active={activeTab === "memory"}
                    onClick={() => setActiveTab("memory")}
                />
                <button
                    onClick={() => setCollapsed(true)}
                    className="px-3 text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-tertiary)] transition-colors"
                    title="Collapse panel"
                    aria-label="Collapse right rail"
                >
                    <X size={16} />
                </button>
            </div>
            <div className={taskCount > 0 ? "flex-1 min-h-0 overflow-hidden flex flex-col" : "flex-1 overflow-hidden"}>
                {activeTab === "tools" && <ToolsPanel />}
                {activeTab === "memory" && (
                    <MemoryPanel
                        memories={recentMemories}
                        onClose={() => setActiveTab("tools")}
                    />
                )}
            </div>
            {taskCount > 0 && (
                <div className="border-t border-[var(--color-border-primary)] h-2/5 min-h-0 shrink-0 flex">
                    <TasksPanel
                        conversationId={conversationId}
                        onClose={() => { /* Tasks slab auto-hides when empty; no manual close */ }}
                    />
                </div>
            )}
        </div>
    );
```

Note: `TasksPanel` is rendered inside a flex container with `h-2/5`. The existing `TasksPanel` styles its own root with `w-80` — the slab wrapper here will let it fill the rail width. Verify this in step 4; if the inner panel still tries to take its own width, a small inline style override (`style={{ width: "100%" }}` on the wrapper, or replacing `w-80` with `w-full` inside the slab) will be needed. Flag any adjustment in the commit message.

- [ ] **Step 3: Type-check**

Run: `npx tsc --noEmit`
Expected: clean.

- [ ] **Step 4: Run the app and verify**

Run: `npm run tauri dev`

Verify:
- With a conversation that has no task list: rail looks the same as Task 4 — no slab at bottom.
- Trigger the model to create a task list (ask it to do something multi-step that uses the built-in `update_tasks` tool, or invoke the relevant tool manually if you have one wired up). Expected: a Tasks slab appears at the bottom of the rail showing the task list, and the panel above shrinks to share the height.
- Have the model (or manually) clear the task list. Expected: the slab unmounts and the panel above reclaims the full height.
- If the slab width looks wrong (TasksPanel rendering at fixed 320px inside a flex container), see the note in step 2.

- [ ] **Step 5: Commit**

```bash
git add src/components/RightRail.tsx src/App.tsx
git commit -m "feat(rail): Tasks slab auto-shows when a task list exists"
```

---

## Task 6: Trim left activity bar + remove old SidePanel plumbing

**Goal:** Drop the Memory, Tasks, and Tools (wrench) buttons from the activity bar. Remove `showTools`, `activeSidePanel`, `setActiveSidePanel` state. Remove `SidePanel` type. Remove `activeSidePanel`/`onChangeSidePanel` props from ChatInterface. Remove the inline `<MemoryPanel />` / `<TasksPanel />` renders from ChatInterface.

**Files:**
- Modify: `src/App.tsx`
- Modify: `src/components/ChatInterface.tsx`

- [ ] **Step 1: Trim activity bar buttons and state in App.tsx**

In `src/App.tsx`:

Remove the `SidePanel` type export (line 18):

```tsx
export type SidePanel = "none" | "memory" | "tasks";
```

Remove these state hooks (currently near line 23-24):

```tsx
  const [showTools, setShowTools] = useState(true);
  const [activeSidePanel, setActiveSidePanel] = useState<SidePanel>("none");
```

Remove the `openSidePanel` helper (currently around lines 91-96):

```tsx
  const openSidePanel = (panel: Exclude<SidePanel, "none">) => {
    if (activeTab !== "chat") setActiveTab("chat");
    setActiveSidePanel(activeSidePanel === panel ? "none" : panel);
  };
```

Remove the Brain (memory) button (currently around lines 130-142):

```tsx
        <button
          onClick={() => openSidePanel("memory")}
          ...
        >
          <Brain size={20} />
          ...
        </button>
```

Remove the ListChecks (tasks) button (currently around lines 144-150):

```tsx
        <button
          onClick={() => openSidePanel("tasks")}
          ...
        >
          <ListChecks size={20} />
        </button>
```

Remove the Wrench (tools) button (currently around lines 160-166):

```tsx
        <button
          onClick={() => setShowTools(!showTools)}
          ...
        >
          <Wrench size={20} />
        </button>
```

Update the lucide-react import — remove `Brain`, `ListChecks`, `Wrench`:

```tsx
import { Eye, EyeOff, MessageSquare, Settings } from "lucide-react";
```

Update the `<ChatInterface />` call to drop the two side-panel props (currently lines 187-192):

```tsx
            <ChatInterface
              conversationId={activeConvId}
              recentMemories={recentMemories}
            />
```

- [ ] **Step 2: Remove inline Memory/Tasks panel renders and props from ChatInterface.tsx**

In `src/components/ChatInterface.tsx`:

Remove the imports (around lines 14-15):

```tsx
import MemoryPanel from "./MemoryPanel";
import TasksPanel from "./TasksPanel";
```

Find and remove the `SidePanel` import. It comes from App.tsx via something like:

```tsx
import { SidePanel } from "../App";
```

(Use `grep -n "SidePanel" src/components/ChatInterface.tsx` to locate the exact import line if it's structured differently.)

Remove the two props from the props interface (around line 81):

```tsx
    activeSidePanel: SidePanel;
    onChangeSidePanel: (panel: SidePanel) => void;
```

Update the destructure in the component signature (around line 113):

```tsx
export default function ChatInterface({ conversationId, recentMemories }: ChatInterfaceProps) {
```

Remove the inline panel renders (around lines 1616-1627):

```tsx
            {activeSidePanel === 'memory' && (
                <MemoryPanel
                    memories={recentMemories}
                    onClose={() => onChangeSidePanel('none')}
                />
            )}
            {activeSidePanel === 'tasks' && (
                <TasksPanel
                    conversationId={conversationId}
                    onClose={() => onChangeSidePanel('none')}
                />
            )}
```

Search the rest of the file for any other reference to `activeSidePanel` or `onChangeSidePanel` and remove or adapt:

```bash
grep -n "activeSidePanel\|onChangeSidePanel" src/components/ChatInterface.tsx
```

If the only remaining references are the ones removed above, you're done. Otherwise inspect each — it may be safe to delete or it may indicate the props serve some other purpose (in which case flag that for the user before deleting).

- [ ] **Step 3: Remove the memory badge logic that lived next to the Brain button**

In Task 6 Step 1 we removed the Brain button which contained badge logic for `recentMemories.length > 0`. That logic is gone. The `recentMemories` state in App.tsx is still used (passed to RightRail), so don't remove the listener.

If you want a visual badge on the Memory tab itself, that's out of scope for v1 — flag as a follow-up.

- [ ] **Step 4: Type-check**

Run: `npx tsc --noEmit`
Expected: clean.

- [ ] **Step 5: Run the app and verify**

Run: `npm run tauri dev`

Verify:
- Left activity bar shows ONLY: app icon, Chat, Inspect Context (Eye/EyeOff toggle), Settings.
- No Memory (Brain) button.
- No Tasks (ListChecks) button.
- No Tools (Wrench) button.
- Right rail still works exactly as before (Tools/Memory tabs, close button, tasks slab).
- No console errors or warnings.

- [ ] **Step 6: Commit**

```bash
git add src/App.tsx src/components/ChatInterface.tsx
git commit -m "refactor(rail): drop legacy side-panel plumbing from App and ChatInterface"
```

---

## Task 7: Final verification pass

**Goal:** End-to-end manual smoke. Ensure all the journeys work and there's no dead code or stale references.

**Files:** none modified unless issues found.

- [ ] **Step 1: Grep for orphan references**

Run each and verify zero results (or only results that are clearly intentional, e.g. inside the right rail itself):

```bash
grep -rn "activeSidePanel" src/
grep -rn "onChangeSidePanel" src/
grep -rn "showTools" src/
grep -rn "SidePanel" src/
grep -rn '"none" | "memory" | "tasks"' src/
```

Expected: no matches in `src/App.tsx` or `src/components/ChatInterface.tsx`. The new `RightRailTab = "tools" | "memory"` is fine and lives only inside `RightRail.tsx`.

- [ ] **Step 2: Full type check**

Run: `npx tsc --noEmit`
Expected: clean.

- [ ] **Step 3: Run the app and do a full smoke walk**

Run: `npm run tauri dev`

Walk through:

1. App opens with right rail visible, Tools tab active.
2. Switch to Memory tab → MemoryPanel renders with Context/Facts/Docs sub-tabs.
3. Switch back to Tools.
4. Click `[×]` → rail collapses to thin strip.
5. Click Wrench icon in strip → rail expands to Tools.
6. Click Brain icon in strip → rail expands to Memory.
7. Reload (or quit + relaunch) → rail state is preserved.
8. Switch to Settings tab in left bar → right rail still visible (this is new behavior, intentional per spec).
9. Switch back to Chat.
10. Trigger a task list (ask the model to do a multi-step task) → Tasks slab appears at the bottom of the rail.
11. Wait for tasks to complete or clear them → slab disappears.

- [ ] **Step 4: Final commit if anything was adjusted in this pass**

If steps 1-3 found nothing to fix, no commit needed. If a fix was made:

```bash
git add <files>
git commit -m "fix(rail): <what was fixed>"
```

---

## Self-review

Checked against the spec:

- **Layout** (spec §1) — implemented in Tasks 1-3 and Task 5.
- **Left activity bar trimmed to chat / settings / inspect-context** (spec §2) — Task 6 step 1. (Also drops the Tools/Wrench button, which the spec didn't explicitly mention but follows from the design intent: the activity bar list is exclusive, not a delta.)
- **State model** (spec §3) — Tasks 2-4. `RightRailState` is in `localStorage` per spec.
- **Components** (spec §4) — new RightRail in Task 1, composes existing panels unchanged.
- **Default state on first launch** (spec §5) — Task 4's `DEFAULT_STATE` covers it.
- **Reopening from collapsed strip** (spec §5) — Task 3 each CollapsedIcon sets activeTab AND uncollapses.
- **Tasks slab height ~40%** (spec §5) — Task 5 uses `h-2/5`.
- **Tasks empty transition** (spec §5) — Task 5 unmounts the slab when `taskCount === 0`. No animation; spec says it's optional.
- **Persistence** (spec §5) — Task 4.
- **Settings tab still shows rail** (spec §5) — Task 1 step 2 places the rail outside both tab divs.
- **Expanded rail width ~320px** (spec §5) — Task 1 uses `w-80`.
- **Cleanup** (spec §6) — Task 6 covers all named files and Task 7 step 1 greps for orphans.
- **Out of scope** (spec §7) — drag-to-resize, cross-machine sync, animated slab, per-conversation rail state — none of these tasks attempt them.

Placeholder/ambiguity sweep — no TBDs, no "implement later," every step has either exact code or exact verification criteria. Type signatures match across tasks (`RightRailTab`, `RightRailState`, `RightRailProps` all introduced and used consistently).

---

Plan complete.
