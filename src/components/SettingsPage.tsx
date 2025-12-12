import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Server, Plus } from "lucide-react";
import ProvidersSettings, { ProviderConfig } from "./ProvidersSettings";

export default function SettingsPage() {
    const [servers, setServers] = useState<{ name: string, status: 'connected' | 'error', config: any }[]>([]);
    const [name, setName] = useState("");
    const [command, setCommand] = useState("");
    const [args, setArgs] = useState("");

    const [status, setStatus] = useState("");
    const [providers, setProviders] = useState<Record<string, ProviderConfig>>({});
    const [fullSettings, setFullSettings] = useState<any>({});

    useEffect(() => {
        loadServers();
    }, []);

    const loadServers = async () => {
        try {
            const active: string[] = await invoke("get_mcp_servers");
            const settings: any = await invoke("get_settings");
            setFullSettings(settings);

            const allServers = Object.entries(settings.mcp_servers || {}).map(([key, val]) => ({
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
            // Merge providers back into settings object
            const newSettings = { ...fullSettings, providers };
            await invoke("save_settings", { settings: newSettings });
            setStatus("Settings Saved!");
            setTimeout(() => setStatus(""), 2000);
        } catch (e: any) {
            setStatus("Error saving: " + e);
        }
    };

    const handleAdd = async () => {
        if (!name || !command) return;
        setStatus("Adding...");
        try {
            // Very basic arg parsing, splits by comma
            const argList = args.split(",").map(s => s.trim()).filter(s => s);
            await invoke("add_mcp_server", {
                name,
                command,
                args: argList,
                env: {}
            });
            setStatus("Added!");
            setName("");
            setCommand("");
            setArgs("");
            loadServers();
        } catch (e: any) {
            setStatus("Error: " + e);
        }
    };

    return (
        <div className="p-6 bg-gray-950 h-full text-white overflow-auto font-sans">
            <h2 className="text-2xl font-bold mb-6 flex items-center gap-2 text-white">
                <Server className="text-blue-500" /> MCP Servers
            </h2>

            <div className="grid gap-4 mb-8">
                {servers.map(s => (
                    <div key={s.name} className={`bg-gray-900 p-4 rounded-lg border flex justify-between items-center shadow-lg ${s.status === 'connected' ? 'border-gray-800' : 'border-red-900/50 bg-red-950/10'}`}>
                        <div className="flex items-center gap-3">
                            <div className={`w-2 h-2 rounded-full ${s.status === 'connected' ? 'bg-green-500 animate-pulse' : 'bg-red-500'}`} />
                            <span className="font-mono font-bold text-gray-200">{s.name}</span>
                        </div>
                        <div className={`text-xs font-bold px-2 py-1 rounded ${s.status === 'connected' ? 'text-green-500 bg-green-500/10' : 'text-red-400 bg-red-500/10'}`}>
                            {s.status.toUpperCase()}
                        </div>
                    </div>
                ))}
                {servers.length === 0 && (
                    <div className="bg-gray-900/50 p-8 rounded-lg border border-dashed border-gray-800 text-center">
                        <Server className="w-12 h-12 text-gray-700 mx-auto mb-3" />
                        <p className="text-gray-500">No MCP servers connected.</p>
                    </div>
                )}
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

            <h3 className="text-lg font-bold mb-4 flex items-center gap-2">
                <Plus className="w-5 h-5 text-blue-500" /> Add New Server
            </h3>
            <div className="space-y-4">
                <div>
                    <label className="block text-xs font-bold text-gray-500 uppercase tracking-wider mb-1">Server Name</label>
                    <input
                        className="w-full bg-gray-950 border border-gray-700 rounded-lg p-3 text-sm focus:border-blue-500 focus:ring-1 focus:ring-blue-500 outline-none transition-all"
                        value={name}
                        onChange={e => setName(e.target.value)}
                        placeholder="e.g. filesystem"
                    />
                </div>
                <div>
                    <label className="block text-xs font-bold text-gray-500 uppercase tracking-wider mb-1">Command</label>
                    <input
                        className="w-full bg-gray-950 border border-gray-700 rounded-lg p-3 text-sm font-mono focus:border-blue-500 focus:ring-1 focus:ring-blue-500 outline-none transition-all"
                        value={command}
                        onChange={e => setCommand(e.target.value)}
                        placeholder="e.g. npx"
                    />
                </div>
                <div>
                    <label className="block text-xs font-bold text-gray-500 uppercase tracking-wider mb-1">Arguments (comma separated)</label>
                    <input
                        className="w-full bg-gray-950 border border-gray-700 rounded-lg p-3 text-sm font-mono focus:border-blue-500 focus:ring-1 focus:ring-blue-500 outline-none transition-all"
                        value={args}
                        onChange={e => setArgs(e.target.value)}
                        placeholder="-y, @modelcontextprotocol/server-filesystem, /path/to/dir"
                    />
                </div>
                <button
                    onClick={handleAdd}
                    className="w-full bg-blue-600 hover:bg-blue-500 text-white font-bold py-3 rounded-lg transition-all shadow-lg shadow-blue-600/20 active:scale-95"
                >
                    Connect Server
                </button>
                {status && (
                    <div className={`text-sm mt-3 p-3 rounded ${status.startsWith("Error") ? "bg-red-500/10 text-red-400" : "bg-green-500/10 text-green-400"}`}>
                        {status}
                    </div>
                )}
            </div>
        </div>

    );
}
