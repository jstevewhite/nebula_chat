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
