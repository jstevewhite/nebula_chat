import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Server, Plus, Edit2, Book } from "lucide-react";
import ProvidersSettings, { ProviderConfig } from "./ProvidersSettings";
import PromptsSettings from "./PromptsSettings";

export default function SettingsPage() {
    const [servers, setServers] = useState<{ name: string, status: 'connected' | 'error', config: any }[]>([]);
    const [providers, setProviders] = useState<Record<string, ProviderConfig>>({});
    const [fullSettings, setFullSettings] = useState<any>({});
    const [status, setStatus] = useState("");

    // Modal State
    const [isModalOpen, setIsModalOpen] = useState(false);
    const [editingServer, setEditingServer] = useState<string | null>(null);

    // Form State
    const [name, setName] = useState("");
    const [transportType, setTransportType] = useState<"stdio" | "sse">("stdio");
    const [command, setCommand] = useState("");
    const [args, setArgs] = useState("");
    const [url, setUrl] = useState("");

    useEffect(() => {
        loadServers();
    }, []);

    const loadServers = async () => {
        try {
            const active: string[] = await invoke("get_mcp_servers");
            const settings: any = await invoke("get_settings");
            setFullSettings(settings);

            const allServers = Object.entries(settings.mcp_servers || {}).map(([key, val]: [string, any]) => ({
                name: key,
                status: active.includes(key) ? 'connected' : 'error',
                config: val
            }));

            setServers(allServers as any);
            setProviders(settings.providers || {});
        } catch (e) {
            console.error(e);
        }
    };

    const saveSettings = async () => {
        try {
            const newSettings = { ...fullSettings, providers };
            await invoke("save_settings", { settings: newSettings });
            setStatus("Settings Saved!");
            setTimeout(() => setStatus(""), 2000);
        } catch (e: any) {
            setStatus("Error saving: " + e);
        }
    };

    const openAddModal = () => {
        setEditingServer(null);
        setName("");
        setTransportType("stdio");
        setCommand("");
        setArgs("");
        setUrl("");
        setStatus("");
        setIsModalOpen(true);
    };

    const openEditModal = (serverName: string, config: any) => {
        setEditingServer(serverName);
        setName(serverName);

        // Flattened config checks
        if (config.type === "Sse") {
            setTransportType("sse");
            setUrl(config.url || "");
            setCommand("");
            setArgs("");
        } else {
            // Default to Stdio
            setTransportType("stdio");
            setCommand(config.command || "");
            setArgs(config.args ? config.args.join(", ") : "");
            setUrl("");
        }

        setStatus("");
        setIsModalOpen(true);
    };

    const handleSaveServer = async () => {
        if (!name) return;
        setStatus("Saving...");

        try {
            // Construct config with flattened structure
            let newConfig: any = {
                auto_approve: false
            };

            if (transportType === "stdio") {
                newConfig.type = "Stdio";
                newConfig.command = command;
                newConfig.args = args.split(",").map(s => s.trim()).filter(s => s);
                newConfig.env = {};
            } else {
                newConfig.type = "Sse";
                newConfig.url = url;
            }

            if (editingServer) {
                // Edit
                await invoke("edit_mcp_server", {
                    originalName: editingServer,
                    newConfig
                });
            } else {
                // Add
                const argList = args.split(",").map(s => s.trim()).filter(s => s);
                // add_mcp_server takes transportType etc as args, not a full config object
                await invoke("add_mcp_server", {
                    name,
                    transportType,
                    command: transportType === "stdio" ? command : null,
                    args: transportType === "stdio" ? argList : null,
                    url: transportType === "sse" ? url : null
                });
            }

            setStatus("Success!");
            setIsModalOpen(false);
            loadServers();
        } catch (e: any) {
            setStatus("Error: " + e);
        }
    };

    return (
        <div className="p-6 bg-gray-950 h-full text-white overflow-auto font-sans relative">
            <h2 className="text-2xl font-bold mb-6 flex items-center gap-2 text-white">
                <Book className="text-blue-500" /> System Prompts
            </h2>
            <div className="mb-10">
                <PromptsSettings />
            </div>

            <div className="bg-gray-900 p-6 rounded-xl border border-gray-800 shadow-xl mb-8">
                <h3 className="text-lg font-bold mb-4 flex items-center gap-2">
                    <Book className="w-5 h-5 text-purple-500" /> Intelligence Settings
                </h3>

                <div className="mb-4">
                    <label className="block text-sm font-bold text-gray-400 mb-2">
                        Memory Strategy Model
                    </label>
                    <p className="text-xs text-gray-500 mb-2">
                        Select a model to use for analyzing and summarizing retrieved memories.
                        A smaller model (e.g., Llama 3 8B) is recommended for speed.
                    </p>
                    <select
                        value={fullSettings.context_model || ""}
                        onChange={(e) => setFullSettings({ ...fullSettings, context_model: e.target.value })}
                        className="w-full bg-gray-950 border border-gray-700 rounded-lg p-3 text-sm focus:border-purple-500 focus:ring-1 focus:ring-purple-500 outline-none text-white"
                        style={{ backgroundColor: '#030712', color: 'white' }}
                    >
                        <option value="" style={{ backgroundColor: '#030712', color: 'white' }}>None (Raw Injection)</option>
                        {Object.entries(providers).flatMap(([pkey, pval]) =>
                            pval.models.filter(m => m.visible).map(m => (
                                <option key={`${pkey}::${m.id}`} value={`${pkey}::${m.id}`} style={{ backgroundColor: '#030712', color: 'white' }}>
                                    {pkey} - {m.name}
                                </option>
                            ))
                        )}
                    </select>
                </div>
            </div>

            <h2 className="text-2xl font-bold mb-6 flex items-center gap-2 text-white">
                <Server className="text-blue-500" /> MCP Servers
            </h2>

            <div className="grid gap-4 mb-8">
                {servers.map(s => (
                    <div key={s.name} className={`bg-gray-900 p-4 rounded-lg border flex justify-between items-center shadow-lg ${s.status === 'connected' ? 'border-gray-800' : 'border-red-900/50 bg-red-950/10'}`}>
                        <div className="flex items-center gap-3">
                            <div className={`w-2 h-2 rounded-full ${s.status === 'connected' ? 'bg-green-500 animate-pulse' : 'bg-red-500'}`} />
                            <div className="flex flex-col">
                                <span className="font-mono font-bold text-gray-200">{s.name}</span>
                                <span className="text-xs text-gray-500">
                                    {s.config.type === 'Sse' ? 'SSE' : 'Stdio'}
                                    {s.config.type === 'Sse' ? ` (${s.config.url})` : ` (${s.config.command})`}
                                </span>
                            </div>
                        </div>
                        <div className="flex items-center gap-2">
                            <div className={`text-xs font-bold px-2 py-1 rounded ${s.status === 'connected' ? 'text-green-500 bg-green-500/10' : 'text-red-400 bg-red-500/10'}`}>
                                {s.status.toUpperCase()}
                            </div>
                            <button
                                onClick={() => openEditModal(s.name, s.config)}
                                className="p-2 hover:bg-gray-800 rounded-lg text-gray-400 hover:text-white transition-colors"
                            >
                                <Edit2 size={16} />
                            </button>
                        </div>
                    </div>
                ))}

                <button
                    onClick={openAddModal}
                    className="w-full py-4 border-2 border-dashed border-gray-800 rounded-xl text-gray-500 hover:border-gray-600 hover:text-gray-300 transition-colors flex items-center justify-center gap-2 font-semibold"
                >
                    <Plus className="w-5 h-5" /> Add MCP Server
                </button>
            </div>

            <div className="bg-gray-900 p-6 rounded-xl border border-gray-800 shadow-xl mb-8">
                <h3 className="text-lg font-bold mb-4 flex items-center gap-2">
                    <Plus className="w-5 h-5 text-blue-500" /> Model Providers
                </h3>

                <ProvidersSettings providers={providers} onChange={setProviders} />

                <div className="mt-6 flex justify-end">
                    <button
                        onClick={saveSettings}
                        className="bg-blue-600 hover:bg-blue-500 text-white font-bold py-2 px-6 rounded-lg transition-all shadow-lg shadow-blue-500/20"
                    >
                        Save Configuration
                    </button>
                </div>
            </div>

            {/* Modal */}
            {
                isModalOpen && (
                    <div className="fixed inset-0 bg-black/80 flex items-center justify-center z-50 p-4">
                        <div className="bg-gray-900 border border-gray-700 rounded-xl w-full max-w-lg shadow-2xl overflow-hidden">
                            <div className="p-6 border-b border-gray-800 flex justify-between items-center">
                                <h3 className="text-xl font-bold">{editingServer ? 'Edit Server' : 'Add Server'}</h3>
                                <button onClick={() => setIsModalOpen(false)} className="text-gray-500 hover:text-white">&times;</button>
                            </div>

                            <div className="p-6 space-y-4">
                                {!editingServer && (
                                    <div>
                                        <label className="block text-xs font-bold text-gray-500 uppercase tracking-wider mb-1">Server Name</label>
                                        <input
                                            className="w-full bg-gray-950 border border-gray-700 rounded-lg p-3 text-sm focus:border-blue-500 focus:ring-1 focus:ring-blue-500 outline-none"
                                            value={name}
                                            onChange={e => setName(e.target.value)}
                                            placeholder="e.g. filesystem"
                                        />
                                    </div>
                                )}

                                <div>
                                    <label className="block text-xs font-bold text-gray-500 uppercase tracking-wider mb-1">Transport Type</label>
                                    <div className="flex gap-2">
                                        <button
                                            onClick={() => setTransportType("stdio")}
                                            className={`flex-1 py-2 rounded-lg text-sm font-bold border ${transportType === "stdio" ? "bg-blue-600 border-blue-600 text-white" : "bg-gray-950 border-gray-700 text-gray-400 hover:bg-gray-800"}`}
                                        >
                                            Stdio (Local)
                                        </button>
                                        <button
                                            onClick={() => setTransportType("sse")}
                                            className={`flex-1 py-2 rounded-lg text-sm font-bold border ${transportType === "sse" ? "bg-blue-600 border-blue-600 text-white" : "bg-gray-950 border-gray-700 text-gray-400 hover:bg-gray-800"}`}
                                        >
                                            SSE (Remote)
                                        </button>
                                    </div>
                                </div>

                                {transportType === "stdio" ? (
                                    <>
                                        <div>
                                            <label className="block text-xs font-bold text-gray-500 uppercase tracking-wider mb-1">Command</label>
                                            <input
                                                className="w-full bg-gray-950 border border-gray-700 rounded-lg p-3 text-sm font-mono focus:border-blue-500 focus:ring-1 focus:ring-blue-500 outline-none"
                                                value={command}
                                                onChange={e => setCommand(e.target.value)}
                                                placeholder="e.g. npx"
                                            />
                                        </div>
                                        <div>
                                            <label className="block text-xs font-bold text-gray-500 uppercase tracking-wider mb-1">Arguments</label>
                                            <input
                                                className="w-full bg-gray-950 border border-gray-700 rounded-lg p-3 text-sm font-mono focus:border-blue-500 focus:ring-1 focus:ring-blue-500 outline-none"
                                                value={args}
                                                onChange={e => setArgs(e.target.value)}
                                                placeholder="-y, @modelcontextprotocol/server..."
                                            />
                                        </div>
                                    </>
                                ) : (
                                    <div>
                                        <label className="block text-xs font-bold text-gray-500 uppercase tracking-wider mb-1">SSE URL</label>
                                        <input
                                            className="w-full bg-gray-950 border border-gray-700 rounded-lg p-3 text-sm font-mono focus:border-blue-500 focus:ring-1 focus:ring-blue-500 outline-none"
                                            value={url}
                                            onChange={e => setUrl(e.target.value)}
                                            placeholder="http://localhost:3000/sse"
                                        />
                                    </div>
                                )}

                                {status && (
                                    <div className={`text-sm p-3 rounded ${status.startsWith("Error") ? "bg-red-500/10 text-red-400" : "bg-green-500/10 text-green-400"}`}>
                                        {status}
                                    </div>
                                )}
                            </div>

                            <div className="p-6 border-t border-gray-800 flex justify-end gap-2">
                                <button
                                    onClick={() => setIsModalOpen(false)}
                                    className="px-4 py-2 rounded-lg hover:bg-gray-800 text-gray-400 font-bold text-sm"
                                >
                                    Cancel
                                </button>
                                <button
                                    onClick={handleSaveServer}
                                    className="px-6 py-2 rounded-lg bg-blue-600 hover:bg-blue-500 text-white font-bold text-sm shadow-lg shadow-blue-600/20"
                                >
                                    {editingServer ? 'Save Changes' : 'Connect Server'}
                                </button>
                            </div>
                        </div>
                    </div>
                )
            }
        </div >
    );
}
