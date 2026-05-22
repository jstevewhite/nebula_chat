import ToolsPanel from "./ToolsPanel";

export default function RightRail() {
    return (
        <div className="w-80 h-full border-l border-[var(--color-border-primary)] bg-[var(--color-bg-secondary)] flex flex-col shrink-0">
            <div className="flex-1 overflow-hidden">
                <ToolsPanel />
            </div>
        </div>
    );
}
