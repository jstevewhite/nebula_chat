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
