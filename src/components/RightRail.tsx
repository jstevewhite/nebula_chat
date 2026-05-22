import { useState, useEffect } from "react";
import { Wrench, Brain, X } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import ToolsPanel from "./ToolsPanel";
import MemoryPanel from "./MemoryPanel";
import TasksPanel from "./TasksPanel";
import type { PersistedTask } from "./TasksPanel";

type RightRailTab = "tools" | "memory";

interface RightRailProps {
    recentMemories: string[];
    conversationId: string | null;
}

export default function RightRail({ recentMemories, conversationId }: RightRailProps) {
    const [activeTab, setActiveTab] = useState<RightRailTab>(() => loadState().activeTab);
    const [collapsed, setCollapsed] = useState<boolean>(() => loadState().collapsed);

    useEffect(() => {
        saveState({ activeTab, collapsed });
    }, [activeTab, collapsed]);

    const [tasks, setTasks] = useState<PersistedTask[]>([]);

    useEffect(() => {
        if (!conversationId) {
            setTasks([]);
            return;
        }
        let cancelled = false;
        invoke<PersistedTask[]>("get_conversation_tasks", { conversationId })
            .then((rows) => {
                if (!cancelled) setTasks(rows);
            })
            .catch((e) => console.error("RightRail: get_conversation_tasks failed", e));
        return () => {
            cancelled = true;
        };
    }, [conversationId]);

    useEffect(() => {
        const unlistenPromise = listen<{ conversation_id: string; tasks: PersistedTask[] }>(
            "tasks-updated",
            (event) => {
                if (event.payload.conversation_id === conversationId) {
                    setTasks(event.payload.tasks);
                }
            }
        );
        return () => {
            unlistenPromise.then((unlisten) => unlisten());
        };
    }, [conversationId]);

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
            <div className={tasks.length > 0 ? "flex-1 min-h-0 overflow-hidden flex flex-col" : "flex-1 overflow-hidden"}>
                {activeTab === "tools" && <ToolsPanel />}
                {activeTab === "memory" && (
                    <MemoryPanel memories={recentMemories} />
                )}
            </div>
            {tasks.length > 0 && (
                <div className="border-t border-[var(--color-border-primary)] h-2/5 min-h-0 shrink-0 flex">
                    <TasksPanel
                        conversationId={conversationId}
                        tasks={tasks}
                    />
                </div>
            )}
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
