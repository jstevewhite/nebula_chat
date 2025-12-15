import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Wrench, CheckCircle2, XCircle, RefreshCw, Search, ChevronDown, ChevronRight, CheckSquare, Square } from "lucide-react";

interface ToolStatus {
    name: String;
    description: String;
    server: String;
    enabled: boolean;
}

export default function ToolsPanel() {
    const [tools, setTools] = useState<ToolStatus[]>([]);
    const [loading, setLoading] = useState(false);
    const [searchQuery, setSearchQuery] = useState("");
    const [expandedServers, setExpandedServers] = useState<Set<string>>(new Set());

    const loadTools = async () => {
        setLoading(true);
        try {
            const list = await invoke<ToolStatus[]>("get_tools");
            setTools(list);
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

    const toggleTool = async (name: string, enabled: boolean) => {
        setTools(tools.map(t => t.name === name ? { ...t, enabled } : t));
        try {
            await invoke("toggle_tool", { toolName: name, enabled });
        } catch (e) {
            console.error(e);
            loadTools();
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
        <div className="h-full flex flex-col bg-gray-950 border-l border-gray-800 text-white w-80">
            <div className="p-4 border-b border-gray-800 flex items-center justify-between">
                <div className="flex items-center gap-2 font-semibold">
                    <Wrench size={18} className="text-gray-400" />
                    <span>Tools</span>
                </div>
                <button
                    onClick={loadTools}
                    className={`p-1.5 rounded hover:bg-gray-800 text-gray-400 hover:text-white transition-colors ${loading ? "animate-spin" : ""}`}
                    title="Refresh Tools"
                >
                    <RefreshCw size={14} />
                </button>
            </div>

            <div className="p-3 border-b border-gray-800">
                <div className="relative">
                    <Search className="absolute left-2.5 top-2.5 text-gray-500 w-3.5 h-3.5" />
                    <input
                        type="text"
                        placeholder="Filter tools..."
                        value={searchQuery}
                        onChange={(e) => setSearchQuery(e.target.value)}
                        className="w-full bg-gray-900 border border-gray-800 rounded-lg py-1.5 pl-8 pr-3 text-xs text-gray-200 focus:outline-none focus:border-blue-500/50 transition-colors placeholder-gray-600"
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
                            <div className="flex items-center justify-between px-2 py-1 bg-gray-900/30 rounded hover:bg-gray-900 transition-colors group select-none">
                                <div
                                    className="flex items-center gap-2 cursor-pointer flex-1 py-1"
                                    onClick={() => toggleServerCollapse(server)}
                                >
                                    {isExpanded ? <ChevronDown size={14} className="text-gray-500" /> : <ChevronRight size={14} className="text-gray-500" />}
                                    <div className="text-xs font-bold text-gray-400 uppercase tracking-wider">
                                        {server}
                                    </div>
                                    <span className="text-[10px] bg-gray-800 px-1.5 rounded-full font-mono text-gray-500">{serverTools.length}</span>
                                </div>
                                <button
                                    onClick={(e) => {
                                        e.stopPropagation();
                                        toggleServerTools(server, serverTools, !allEnabled);
                                    }}
                                    className={`p-1 mr-1 hover:text-white transition-colors ${allEnabled ? "text-green-500" : someEnabled ? "text-green-500/50" : "text-gray-600"}`}
                                    title={allEnabled ? "Disable All" : "Enable All"}
                                >
                                    {allEnabled ? <CheckSquare size={14} /> : someEnabled ? <CheckSquare size={14} className="opacity-50" /> : <Square size={14} />}
                                </button>
                            </div>

                            {isExpanded && (
                                <div className="space-y-0.5 ml-1 pl-2 border-l border-gray-800 animate-in fade-in slide-in-from-top-1 duration-200">
                                    {serverTools.map((tool) => (
                                        <div
                                            key={tool.name as string}
                                            className={`group flex items-start gap-3 p-2 rounded-lg transition-colors border border-transparent ${tool.enabled ? "hover:bg-gray-900/50" : "opacity-60 hover:opacity-80"}`}
                                        >
                                            <button
                                                onClick={() => toggleTool(tool.name as string, !tool.enabled)}
                                                className={`mt-0.5 shrink-0 transition-colors ${tool.enabled ? "text-green-500 hover:text-green-400" : "text-gray-600 hover:text-gray-400"}`}
                                            >
                                                {tool.enabled ? <CheckCircle2 size={16} /> : <XCircle size={16} />}
                                            </button>

                                            <div className="flex-1 min-w-0">
                                                <div className="flex items-center justify-between gap-2">
                                                    <div className="text-sm font-medium text-gray-200 truncate" title={tool.name as string}>
                                                        {(tool.name as string).split("__")[1]}
                                                    </div>
                                                </div>
                                                {tool.description && (
                                                    <div className="text-xs text-gray-500 line-clamp-2 mt-0.5 leading-relaxed">
                                                        {tool.description}
                                                    </div>
                                                )}
                                            </div>
                                        </div>
                                    ))}
                                </div>
                            )}
                        </div>
                    );
                })}
            </div>
        </div>
    );
}
