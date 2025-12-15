import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Server, Plus, Edit2, Book, Trash2 } from "lucide-react";
import ProvidersSettings, { ProviderConfig } from "./ProvidersSettings";
import PromptsSettings from "./PromptsSettings";

export default function SettingsPage() {
    const [servers, setServers] = useState<{ name: string, status: 'connected' | 'error' | 'unknown', config: any }[]>([]);
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
    const [envText, setEnvText] = useState("");
    const [envErrors, setEnvErrors] = useState<string[]>([]);
    const [url, setUrl] = useState("");
    const [allowlist, setAllowlist] = useState("");

    const [denylist, setDenylist] = useState("");
    const [autoApprove, setAutoApprove] = useState(false);

    useEffect(() => {
        loadServers();
    }, []);

    const loadServers = async () => {
        setStatus("Loading settings...");
        try {
            // Load persisted settings first (source of truth for what exists)
            let settings: any = {};
            try {
                settings = await invoke("get_settings");
            } catch (e) {
                console.error("Failed to get settings:", e);
                setStatus(`Error loading settings.json: ${e}`);
                return; // Critical failure
            }

            setFullSettings(settings);
            setProviders(settings.providers || {});

            const baseServers = Object.entries(settings.mcp_servers || {}).map(([key, val]: [string, any]) => ({
                name: key,
                status: 'unknown' as const,
                config: val
            }));
            setServers(baseServers as any);

            // Best-effort runtime status
            try {
                const active: string[] = await invoke("get_mcp_servers");
                const merged = Object.entries(settings.mcp_servers || {}).map(([key, val]: [string, any]) => ({
                    name: key,
                    status: active.includes(key) ? 'connected' : 'error',
                    config: val
                }));
                setServers(merged as any);
            } catch (e) {
                console.error("Failed to get MCP servers:", e);
                setStatus(`Warning: MCP runtime status unavailable: ${e}`);
            }

            // Debug empty settings
            if (!settings.mcp_servers && !settings.providers) {
                setStatus("Warning: Settings appear empty.");
            } else {
                setStatus("");
            }
        } catch (e) {
            console.error(e);
            setStatus("Uncaught error: " + e);
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
        setEnvText("");
        setEnvErrors([]);
        setUrl("");
        setAllowlist("");

        setDenylist("");
        setAutoApprove(false);
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
            setEnvText("");
            setEnvErrors([]);
        } else {
            // Default to Stdio
            setTransportType("stdio");
            setCommand(config.command || "");
            setArgs(config.args ? config.args.join(", ") : "");
            const envObj = (config.env || {}) as Record<string, string>;
            const envLines = Object.entries(envObj)
                .sort(([a], [b]) => a.localeCompare(b))
                .map(([k, v]) => `${k}=${v}`)
                .join("\n");
            setEnvText(envLines);
            setEnvErrors([]);
            setUrl("");
        }

        // Permissions
        const perms = config.permissions || {};
        setAllowlist((perms.allowlist || []).join(", "));

        setDenylist((perms.denylist || []).join(", "));
        setAutoApprove(config.auto_approve || false);

        setStatus("");
        setIsModalOpen(true);
    };

    const handleSaveServer = async () => {
        if (!name) return;
        setStatus("Saving...");

        const parseEnvValidated = (text: string): { env: Record<string, string>, errors: string[] } => {
            const env: Record<string, string> = {};
            const errors: string[] = [];
            const lines = text.split("\n");

            // POSIX env var names are typically [A-Za-z_][A-Za-z0-9_]*
            const keyRe = /^[A-Za-z_][A-Za-z0-9_]*$/;

            lines.forEach((rawLine, idx) => {
                const lineNo = idx + 1;
                const trimmed = rawLine.trim();
                if (!trimmed || trimmed.startsWith("#")) return;

                const eq = trimmed.indexOf("=");
                if (eq <= 0) {
                    errors.push(`Line ${lineNo}: expected KEY=VALUE`);
                    return;
                }

                const key = trimmed.slice(0, eq).trim();
                const value = trimmed.slice(eq + 1); // keep value as-is (can contain '=')

                if (!keyRe.test(key)) {
                    errors.push(`Line ${lineNo}: invalid key '${key}' (use letters/digits/underscore, cannot start with digit)`);
                    return;
                }

                if (value.includes("\u0000")) {
                    errors.push(`Line ${lineNo}: value contains NUL (\\u0000), which is not allowed`);
                    return;
                }

                env[key] = value;
            });

            return { env, errors };
        };

        try {
            // Construct config with flattened structure
            let newConfig: any = {
                auto_approve: autoApprove,
                permissions: {
                    allowlist: allowlist.split(",").map(s => s.trim()).filter(s => s),
                    denylist: denylist.split(",").map(s => s.trim()).filter(s => s)
                }
            };

            if (transportType === "stdio") {
                const parsed = parseEnvValidated(envText);
                setEnvErrors(parsed.errors);
                if (parsed.errors.length > 0) {
                    setStatus("Error: Invalid environment variables.");
                    return;
                }

                newConfig.type = "Stdio";
                newConfig.command = command;
                newConfig.args = args.split(",").map(s => s.trim()).filter(s => s);
                newConfig.env = parsed.env;
            } else {
                setEnvErrors([]);
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
                let env: Record<string, string> | null = null;
                if (transportType === "stdio") {
                    const parsed = parseEnvValidated(envText);
                    setEnvErrors(parsed.errors);
                    if (parsed.errors.length > 0) {
                        setStatus("Error: Invalid environment variables.");
                        return;
                    }
                    env = parsed.env;
                } else {
                    setEnvErrors([]);
                }

                // add_mcp_server takes transportType etc as args, not a full config object
                await invoke("add_mcp_server", {
                    name,
                    transportType,
                    command: transportType === "stdio" ? command : null,
                    args: transportType === "stdio" ? argList : null,
                    env: transportType === "stdio" ? env : null,

                    url: transportType === "sse" ? url : null,
                    auto_approve: autoApprove
                });
            }

            setStatus("Success!");
            setIsModalOpen(false);
            loadServers();
        } catch (e: any) {
            setStatus("Error: " + e);
        }
    };

    const handleDeleteServer = async (serverName: string) => {
        if (!confirm(`Delete MCP server '${serverName}'? This removes it from settings.json.`)) {
            return;
        }
        try {
            setStatus(`Deleting MCP server '${serverName}'...`);
            await invoke("delete_mcp_server", { name: serverName });
            setStatus("Deleted.");
            setTimeout(() => setStatus(""), 1000);
            loadServers();
        } catch (e: any) {
            console.error(e);
            setStatus("Error deleting server: " + e);
        }
    };

    const handleRebuildIndex = async () => {
        if (!confirm("Rebuild memory index? This may take a moment.")) {
            return;
        }
        setStatus("Rebuilding memory index...");
        try {
            await invoke("rebuild_memory_index");
            setStatus("Memory index rebuilt successfully.");
            setTimeout(() => setStatus(""), 2000);
        } catch (e: any) {
            console.error(e);
            setStatus("Error rebuilding index: " + e);
        }
    };

    return (
        <div className="p-6 bg-gray-950 h-full text-white overflow-auto font-sans relative">
            {/* Status Banner */}
            {status && (
                <div className={`mb-6 p-4 rounded-lg flex items-center gap-2 border ${status.includes("Error") || status.includes("Warning") || status.includes("Failed")
                    ? "bg-red-900/20 border-red-500/50 text-red-200"
                    : "bg-blue-900/20 border-blue-500/50 text-blue-200"
                    }`}>
                    <div className="flex-1 font-mono text-sm">{status}</div>
                    <button onClick={() => setStatus("")} className="px-2 hover:bg-white/10 rounded">&times;</button>
                </div>
            )}

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
                    <div className="flex items-center justify-between gap-4 mb-3">
                        <div>
                            <label className="block text-sm font-bold text-gray-400">
                                Long-term Memory
                            </label>
                            <p className="text-xs text-gray-500">
                                Enable retrieval/injection of relevant memories into chats.
                            </p>
                        </div>
                        <label className="flex items-center gap-2 text-sm text-gray-300 select-none">
                            <input
                                type="checkbox"
                                checked={fullSettings.memory_enabled ?? true}
                                onChange={(e) => setFullSettings({ ...fullSettings, memory_enabled: e.target.checked })}
                                className="h-4 w-4 rounded border-gray-700 bg-gray-950"
                            />
                            Enabled
                        </label>
                    </div>

                    <label className="block text-sm font-bold text-gray-400 mb-2">
                        Memory Strategy Model
                    </label>
                    <p className="text-xs text-gray-500 mb-2">
                        Select a model to use for analyzing and summarizing retrieved memories.
                        A smaller model (e.g., Llama 3 8B) is recommended for speed.
                    </p>
                    <select
                        disabled={!(fullSettings.memory_enabled ?? true)}
                        value={fullSettings.context_model || ""}
                        onChange={(e) => setFullSettings({ ...fullSettings, context_model: e.target.value })}
                        className={`w-full border border-gray-700 rounded-lg p-3 text-sm bg-gray-900 text-white focus:border-purple-500 focus:ring-1 focus:ring-purple-500 outline-none intelligence-settings-dropdown ${(fullSettings.memory_enabled ?? true) ? "" : "opacity-50 cursor-not-allowed"}`}
                        style={{ colorScheme: "dark" }}
                    >
                        <option value="">None (Raw Injection)</option>
                        {Object.entries(providers).flatMap(([pkey, pval]) =>
                            pval.models.filter(m => m.visible).map(m => (
                                <option key={`${pkey}::${m.id}`} value={`${pkey}::${m.id}`}>
                                    {pkey} - {m.name}
                                </option>
                            ))
                        )}
                    </select>

                    <div className="mt-4">
                        <label className="block text-sm font-bold text-gray-400 mb-2">
                            Conversation Turns Included
                        </label>
                        <p className="text-xs text-gray-500 mb-2">
                            How many recent turns (user/assistant pairs) to include when deciding which memories are relevant.
                            Set to 0 to disable.
                        </p>
                        <select
                            disabled={!(fullSettings.memory_enabled ?? true)}
                            value={String(fullSettings.context_turns ?? 0)}
                            onChange={(e) => setFullSettings({ ...fullSettings, context_turns: Number(e.target.value) })}
                            className={`w-full border border-gray-700 rounded-lg p-3 text-sm bg-gray-900 text-white focus:border-purple-500 focus:ring-1 focus:ring-purple-500 outline-none intelligence-settings-dropdown ${(fullSettings.memory_enabled ?? true) ? "" : "opacity-50 cursor-not-allowed"}`}
                            style={{ colorScheme: "dark" }}
                        >
                            {[0, 1, 2, 3, 4, 6, 8, 10].map(n => (
                                <option key={n} value={String(n)}>
                                    {n}
                                </option>
                            ))}
                        </select>

                    </div>

                    <div className="mt-6 border-t border-gray-800 pt-4">
                        <button
                            onClick={handleRebuildIndex}
                            disabled={!(fullSettings.memory_enabled ?? true)}
                            className={`w-full py-2 px-4 rounded-lg border border-gray-700 text-sm font-bold transition-colors flex items-center justify-center gap-2
                                ${(fullSettings.memory_enabled ?? true)
                                    ? "bg-gray-800 hover:bg-gray-700 text-gray-300 hover:text-white"
                                    : "opacity-50 cursor-not-allowed text-gray-500"
                                }`}
                        >
                            <Trash2 className="w-4 h-4" /> Rebuild Memory Index
                        </button>
                        <p className="text-xs text-gray-500 mt-2 text-center">
                            Use this if search results seem stale or incorrect.
                        </p>
                    </div>
                </div>
            </div>

            <h2 className="text-2xl font-bold mb-6 flex items-center gap-2 text-white">
                <Server className="text-blue-500" /> MCP Servers
            </h2>

            <div className="grid gap-4 mb-8">
                {servers.map(s => (
                    <div
                        key={s.name}
                        className={`bg-gray-900 p-4 rounded-lg border flex justify-between items-center shadow-lg ${s.status === 'connected'
                            ? 'border-gray-800'
                            : s.status === 'unknown'
                                ? 'border-gray-700/50 bg-gray-950/10'
                                : 'border-red-900/50 bg-red-950/10'
                            }`}
                    >
                        <div className="flex items-center gap-3">
                            <div
                                className={`w-2 h-2 rounded-full ${s.status === 'connected'
                                    ? 'bg-green-500 animate-pulse'
                                    : s.status === 'unknown'
                                        ? 'bg-gray-500'
                                        : 'bg-red-500'
                                    }`}
                            />
                            <div className="flex flex-col">
                                <span className="font-mono font-bold text-gray-200">{s.name}</span>
                                <span className="text-xs text-gray-500">
                                    {s.config.type === 'Sse' ? 'SSE' : 'Stdio'}
                                    {s.config.type === 'Sse' ? ` (${s.config.url})` : ` (${s.config.command})`}
                                </span>
                            </div>
                        </div>
                        <div className="flex items-center gap-2">
                            <div
                                className={`text-xs font-bold px-2 py-1 rounded ${s.status === 'connected'
                                    ? 'text-green-500 bg-green-500/10'
                                    : s.status === 'unknown'
                                        ? 'text-gray-300 bg-gray-500/10'
                                        : 'text-red-400 bg-red-500/10'
                                    }`}
                            >
                                {s.status.toUpperCase()}
                            </div>
                            <button
                                onClick={() => openEditModal(s.name, s.config)}
                                className="p-2 hover:bg-gray-800 rounded-lg text-gray-400 hover:text-white transition-colors"
                                title="Edit"
                            >
                                <Edit2 size={16} />
                            </button>
                            <button
                                onClick={() => handleDeleteServer(s.name)}
                                className="p-2 hover:bg-red-900/30 rounded-lg text-gray-400 hover:text-red-400 transition-colors"
                                title="Delete"
                            >
                                <Trash2 size={16} />
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
                                        <div>
                                            <label className="block text-xs font-bold text-gray-500 uppercase tracking-wider mb-1">Environment (KEY=VALUE)</label>
                                            <textarea
                                                className={`w-full bg-gray-950 border rounded-lg p-3 text-sm font-mono focus:ring-1 outline-none min-h-[120px] ${envErrors.length > 0 ? "border-red-600 focus:border-red-500 focus:ring-red-500" : "border-gray-700 focus:border-blue-500 focus:ring-blue-500"}`}
                                                value={envText}
                                                onChange={e => {
                                                    setEnvText(e.target.value);
                                                    if (envErrors.length) setEnvErrors([]);
                                                }}
                                                placeholder={"FOO=bar\nOPENAI_API_KEY=...\n# comments allowed"}
                                            />
                                            {envErrors.length > 0 && (
                                                <div className="mt-2 text-xs text-red-300 bg-red-900/20 border border-red-700/40 rounded-lg p-2 space-y-1">
                                                    {envErrors.map((err, i) => (
                                                        <div key={i}>{err}</div>
                                                    ))}
                                                </div>
                                            )}
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

                                <div>
                                    <label className="block text-xs font-bold text-gray-500 uppercase tracking-wider mb-1">Allowlist (Comma separated)</label>
                                    <input
                                        className="w-full bg-gray-950 border border-gray-700 rounded-lg p-3 text-sm focus:border-blue-500 focus:ring-1 focus:ring-blue-500 outline-none"
                                        value={allowlist}
                                        onChange={e => setAllowlist(e.target.value)}
                                        placeholder="tool_a, tool_b (Leave empty to allow all)"
                                    />
                                </div>
                                <div>
                                    <label className="block text-xs font-bold text-gray-500 uppercase tracking-wider mb-1">Denylist (Comma separated)</label>
                                    <input
                                        className="w-full bg-gray-950 border border-gray-700 rounded-lg p-3 text-sm focus:border-red-500 focus:ring-1 focus:ring-red-500 outline-none"
                                        value={denylist}
                                        onChange={e => setDenylist(e.target.value)}
                                        placeholder="dangerous_tool, delete_all"
                                    />
                                </div>

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
