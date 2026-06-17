import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Wrench, CheckCircle2, XCircle, RefreshCw, Search, ChevronDown, ChevronRight, CheckSquare, Square, Lock, Unlock, Clock } from "lucide-react";

interface ToolStatus {
    name: String;
    description: String;
    server: String;
    enabled: boolean;
}

const getSimpleToolName = (fullToolName: string): string => {
    const idx = fullToolName.indexOf("__");
    return idx === -1 ? fullToolName : fullToolName.slice(idx + 2);
};

export default function ToolsPanel() {
    const [tools, setTools] = useState<ToolStatus[]>([]);
    const [loading, setLoading] = useState(false);
    const [searchQuery, setSearchQuery] = useState("");
    const [expandedServers, setExpandedServers] = useState<Set<string>>(new Set());
    const [serverPolicies, setServerPolicies] = useState<Record<string, boolean>>({});
    const [toolPolicies, setToolPolicies] = useState<Record<string, boolean>>({});
    // Prompt-cache controls (Phase 2): freezing the tool set protects the cache;
    // the 1h TTL is an Anthropic-only knob.
    const [locked, setLocked] = useState(false);
    const [cacheTtl1h, setCacheTtl1h] = useState(false);

    const loadTools = async () => {
        setLoading(true);
        try {
            const list = await invoke<ToolStatus[]>("get_tools");
            setTools(list);

            // Load tool policies (effective)
            try {
                const effectivePolicies = await invoke<Record<string, boolean>>("get_tool_policies");
                setToolPolicies(effectivePolicies);
            } catch (e) {
                console.warn("Failed to load tool policies", e);
            }

            // Also load server policies (for "Select All" / Inheritance logic)
            try {
                const settings: any = await invoke("get_settings");
                const policies: Record<string, boolean> = {};
                if (settings && settings.mcp_servers) {
                    Object.entries(settings.mcp_servers).forEach(([name, config]: [string, any]) => {
                        policies[name] = config.auto_approve || false;
                    });
                }
                setServerPolicies(policies);
                setLocked(!!settings?.tools_locked);
                setCacheTtl1h(!!settings?.anthropic_cache_ttl_1h);
            } catch (e) {
                console.warn("Failed to load server policies", e);
            }

        } catch (e) {
            console.error(e);
        } finally {
            setLoading(false);
        }
    };

    useEffect(() => {
        loadTools();

        const unlistenPromise = listen("tools-updated", () => {
            console.log("Tools updated event received, reloading tools...");
            loadTools();
        });

        return () => {
            unlistenPromise.then(unlisten => unlisten());
        };
    }, []);

    const toggleServerAutoApprove = async (serverName: string, current: boolean) => {
        // Optimistic update
        setServerPolicies(prev => ({ ...prev, [serverName]: !current }));
        try {
            await invoke("toggle_mcp_server_auto_approve", { serverName, autoApprove: !current });
        } catch (e) {
            console.error(e);
            loadTools(); // revert
        }
    };

    // Update a single boolean setting via read-modify-write (matches the pattern
    // used elsewhere, e.g. ChatInterface's memory toggle).
    const updateSetting = async (key: string, value: boolean) => {
        const current: any = await invoke("get_settings");
        await invoke("save_settings", { settings: { ...current, [key]: value } });
    };

    const toggleLock = async () => {
        const next = !locked;
        setLocked(next); // optimistic
        try {
            await updateSetting("tools_locked", next);
        } catch (e) {
            console.error(e);
            setLocked(!next); // revert
        }
    };

    const toggleCacheTtl = async () => {
        const next = !cacheTtl1h;
        setCacheTtl1h(next); // optimistic
        try {
            await updateSetting("anthropic_cache_ttl_1h", next);
        } catch (e) {
            console.error(e);
            setCacheTtl1h(!next); // revert
        }
    };

    const toggleTool = async (name: string, enabled: boolean) => {
        if (locked) return;
        setTools(tools.map(t => t.name === name ? { ...t, enabled } : t));
        try {
            await invoke("toggle_tool", { toolName: name, enabled });
        } catch (e) {
            console.error(e);
            loadTools();
        }
    };

    const toggleToolAutoApprove = async (fullToolName: string, serverName: string, current: boolean) => {
        // Optimistic update
        setToolPolicies(prev => ({ ...prev, [fullToolName]: !current }));
        const simpleName = getSimpleToolName(fullToolName);
        if (!simpleName) return;

        try {
            await invoke("toggle_tool_auto_approve", {
                serverName,
                toolName: simpleName,
                autoApprove: !current
            });
        } catch (e) {
            console.error(e);
            loadTools(); // revert
        }
    };

    const toggleServerCollapse = (server: string) => {
        const newExpanded = new Set(expandedServers);
        if (newExpanded.has(server)) {
            newExpanded.delete(server);
        } else {
            newExpanded.add(server);
        }
        setExpandedServers(newExpanded);
    };

    const toggleServerTools = async (_server: string, serverTools: ToolStatus[], enable: boolean) => {
        if (locked) return;
        const toolNames = serverTools.map(t => t.name as string);

        // Optimistic Update
        setTools(tools.map(t => toolNames.includes(t.name as string) ? { ...t, enabled: enable } : t));

        try {
            await invoke("toggle_tool_list", { toolNames, enabled: enable });
        } catch (e) {
            console.error(e);
            loadTools();
        }
    };

    // Group tools by server
    const filteredTools = tools.filter(t =>
        t.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
        t.description.toLowerCase().includes(searchQuery.toLowerCase())
    );

    const groupedTools: Record<string, ToolStatus[]> = {};
    filteredTools.forEach(t => {
        const server = t.server as string;
        if (!groupedTools[server]) groupedTools[server] = [];
        groupedTools[server].push(t);
    });

    return (
        <div className="h-full flex flex-col bg-[var(--color-bg-primary)] text-[var(--color-text-primary)]">
            <div className="p-4 border-b border-[var(--color-border-primary)] flex items-center justify-between">
                <div className="flex items-center gap-2 font-semibold">
                    <Wrench size={18} className="text-[var(--color-text-secondary)]" />
                    <span>Tools</span>
                </div>
                <button
                    onClick={loadTools}
                    className={`p-1.5 rounded hover:bg-[var(--color-bg-tertiary)] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors ${loading ? "animate-spin" : ""}`}
                    title="Refresh Tools"
                >
                    <RefreshCw size={14} />
                </button>
            </div>

            {/* Prompt-cache controls (Phase 2) */}
            <div className="px-3 py-2 border-b border-[var(--color-border-primary)] space-y-1.5">
                <button
                    onClick={toggleLock}
                    className={`w-full flex items-center justify-between gap-2 px-2 py-1.5 rounded-lg text-xs transition-colors ${locked ? "bg-amber-500/10 text-amber-400 hover:bg-amber-500/20" : "text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-secondary)]"}`}
                    title={locked
                        ? "Tools are locked — preserves the prompt cache. Click to unlock and allow tool changes."
                        : "Lock tools to preserve the prompt cache. Tools render first in the cached prefix, so toggling one mid-conversation resets the whole cache."}
                >
                    <span className="flex items-center gap-2">
                        {locked ? <Lock size={14} /> : <Unlock size={14} />}
                        <span className="font-medium">{locked ? "Tools locked" : "Lock tools"}</span>
                    </span>
                    <span className="text-[10px] text-[var(--color-text-tertiary)]">preserves prompt cache</span>
                </button>
                <button
                    onClick={toggleCacheTtl}
                    className={`w-full flex items-center justify-between gap-2 px-2 py-1.5 rounded-lg text-xs transition-colors ${cacheTtl1h ? "bg-blue-500/10 text-blue-400 hover:bg-blue-500/20" : "text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-secondary)]"}`}
                    title="Anthropic only: use a 1-hour prompt-cache TTL instead of the 5-minute default. Survives longer idle gaps but doubles the cache-write cost."
                >
                    <span className="flex items-center gap-2">
                        <Clock size={14} />
                        <span className="font-medium">1-hour cache TTL</span>
                    </span>
                    <span className="text-[10px] text-[var(--color-text-tertiary)]">{cacheTtl1h ? "on" : "off"} · Anthropic</span>
                </button>
            </div>

            <div className="p-3 border-b border-[var(--color-border-primary)]">
                <div className="relative">
                    <Search className="absolute left-2.5 top-2.5 text-[var(--color-text-tertiary)] w-3.5 h-3.5" />
                    <input
                        type="text"
                        placeholder="Filter tools..."
                        value={searchQuery}
                        onChange={(e) => setSearchQuery(e.target.value)}
                        className="w-full bg-[var(--color-bg-secondary)] border border-[var(--color-border-primary)] rounded-lg py-1.5 pl-8 pr-3 text-xs text-gray-200 focus:outline-none focus:border-blue-500/50 transition-colors placeholder-gray-600"
                    />
                </div>
            </div>

            <div className="flex-1 overflow-y-auto p-2 space-y-4">
                {Object.keys(groupedTools).length === 0 && !loading && (
                    <div className="text-center text-gray-600 text-sm py-10">
                        No tools found
                    </div>
                )}

                {Object.entries(groupedTools).map(([server, serverTools]) => {
                    const allEnabled = serverTools.every(t => t.enabled);
                    const someEnabled = serverTools.some(t => t.enabled);
                    const isExpanded = expandedServers.has(server);

                    return (
                        <div key={server} className="space-y-1">
                            <div className="flex items-center justify-between px-2 py-1 bg-[var(--color-bg-secondary)]/30 rounded hover:bg-[var(--color-bg-secondary)] transition-colors group select-none">
                                <div
                                    className="flex items-center gap-2 cursor-pointer flex-1 py-1"
                                    onClick={() => toggleServerCollapse(server)}
                                >
                                    {isExpanded ? <ChevronDown size={14} className="text-[var(--color-text-tertiary)]" /> : <ChevronRight size={14} className="text-[var(--color-text-tertiary)]" />}
                                    <div className="text-xs font-bold text-[var(--color-text-secondary)] uppercase tracking-wider">
                                        {server}
                                    </div>
                                    <span className="text-[10px] bg-[var(--color-bg-tertiary)] px-1.5 rounded-full font-mono text-[var(--color-text-tertiary)]">{serverTools.length}</span>
                                </div>

                                <div className="flex items-center gap-1">
                                    <button
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            toggleServerAutoApprove(server, serverPolicies[server] || false);
                                        }}
                                        className={`p-1 hover:text-[var(--color-text-primary)] transition-colors ${serverPolicies[server] ? "text-purple-500" : "text-gray-600 hover:text-purple-400"}`}
                                        title={serverPolicies[server] ? "Auto-Approve Enabled (Click to Disable)" : "Enable Auto-Approve"}
                                    >
                                        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                            <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
                                            {serverPolicies[server] && <path d="M9 12l2 2 4-4" />}
                                        </svg>
                                    </button>
                                    <button
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            toggleServerTools(server, serverTools, !allEnabled);
                                        }}
                                        disabled={locked}
                                        className={`p-1 mr-1 hover:text-[var(--color-text-primary)] transition-colors ${allEnabled ? "text-green-500" : someEnabled ? "text-green-500/50" : "text-gray-600"} ${locked ? "opacity-40 cursor-not-allowed hover:text-current" : ""}`}
                                        title={locked ? "Tools locked — unlock to change" : allEnabled ? "Disable All" : "Enable All"}
                                    >
                                        {allEnabled ? <CheckSquare size={14} /> : someEnabled ? <CheckSquare size={14} className="opacity-50" /> : <Square size={14} />}
                                    </button>
                                </div>
                            </div>

                            {isExpanded && (
                                <div className="space-y-0.5 ml-1 pl-2 border-l border-[var(--color-border-primary)] animate-in fade-in slide-in-from-top-1 duration-200">
                                    {serverTools.map((tool) => {
                                        const isAutoApproved = toolPolicies[tool.name as string] || false;
                                        const isServerLocked = serverPolicies[server] || false;

                                        return (
                                            <div
                                                key={tool.name as string}
                                                className={`group flex items-start gap-3 p-2 rounded-lg transition-colors border border-transparent ${tool.enabled ? "hover:bg-[var(--color-bg-secondary)]/50" : "opacity-60 hover:opacity-80"}`}
                                            >
                                                <button
                                                    onClick={() => toggleTool(tool.name as string, !tool.enabled)}
                                                    disabled={locked}
                                                    title={locked ? "Tools locked — unlock to change" : tool.enabled ? "Disable tool" : "Enable tool"}
                                                    className={`mt-0.5 shrink-0 transition-colors ${tool.enabled ? "text-green-500 hover:text-green-400" : "text-gray-600 hover:text-[var(--color-text-secondary)]"} ${locked ? "opacity-40 cursor-not-allowed" : ""}`}
                                                >
                                                    {tool.enabled ? <CheckCircle2 size={16} /> : <XCircle size={16} />}
                                                </button>

                                                <div className="flex-1 min-w-0">
                                                    <div className="flex items-center justify-between gap-2">
                                                        <div className="text-sm font-medium text-gray-200 truncate" title={tool.name as string}>
                                                            {getSimpleToolName(tool.name as string)}
                                                        </div>

                                                        {/* Tool Auto-Approve Toggle */}
                                                        <button
                                                            onClick={(e) => {
                                                                e.stopPropagation();
                                                                if (!isServerLocked) {
                                                                    toggleToolAutoApprove(tool.name as string, server, isAutoApproved);
                                                                }
                                                            }}
                                                            disabled={isServerLocked}
                                                            className={`p-1 opacity-0 group-hover:opacity-100 transition-opacity ${isAutoApproved
                                                                    ? "opacity-100 text-purple-400"
                                                                    : "text-gray-600 hover:text-purple-400"
                                                                } ${isServerLocked ? "cursor-not-allowed opacity-50" : ""}`}
                                                            title={isServerLocked
                                                                ? "Auto-Approved by Server Policy"
                                                                : isAutoApproved
                                                                    ? "Auto-Approve Enabled"
                                                                    : "Enable Auto-Approve"}
                                                        >
                                                            <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                                                <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
                                                                {isAutoApproved && <path d="M9 12l2 2 4-4" />}
                                                            </svg>
                                                        </button>
                                                    </div>
                                                    {tool.description && (
                                                        <div className="text-xs text-[var(--color-text-tertiary)] line-clamp-2 mt-0.5 leading-relaxed">
                                                            {tool.description}
                                                        </div>
                                                    )}
                                                </div>
                                            </div>
                                        )
                                    })}
                                </div>
                            )}
                        </div>
                    );
                })}
            </div>
        </div>
    );
}
