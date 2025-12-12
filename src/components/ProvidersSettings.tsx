
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Eye, EyeOff, RefreshCw, Trash2, CheckCircle, AlertCircle } from "lucide-react";

export interface ModelConfig {
    id: string;
    name: string;
    visible?: boolean;
}

export type ProviderType = "OpenAI" | "Anthropic" | "Ollama" | "OpenAICompatible";

export interface ProviderConfig {
    enabled: boolean;
    provider_type: ProviderType;
    base_url?: string;
    api_key?: string;
    models: ModelConfig[];
}

interface ProvidersSettingsProps {
    providers: Record<string, ProviderConfig>;
    onChange: (providers: Record<string, ProviderConfig>) => void;
}

export default function ProvidersSettings({ providers, onChange }: ProvidersSettingsProps) {
    const [loading, setLoading] = useState<string | null>(null);

    const updateProvider = (key: string, updates: Partial<ProviderConfig>) => {
        const newProviders = { ...providers };
        if (!newProviders[key]) return;
        newProviders[key] = { ...newProviders[key], ...updates };
        onChange(newProviders);
    };

    const fetchModels = async (key: string) => {
        const p = providers[key];
        if (!p) return;

        setLoading(key);
        try {
            const fetchedModels = await invoke<ModelConfig[]>("fetch_models", {
                providerType: p.provider_type,
                baseUrl: p.base_url,
                apiKey: p.api_key
            });

            // Smart Merge: Keep existing visibility settings
            const currentModelsMap = new Map(p.models.map(m => [m.id, m]));
            const mergedModels = fetchedModels.map(newModel => {
                const existing = currentModelsMap.get(newModel.id);
                return {
                    ...newModel,
                    visible: existing ? existing.visible : true // Default to true if new
                };
            });

            updateProvider(key, { models: mergedModels });
        } catch (e) {
            alert(`Failed to fetch models: ${e}`);
        } finally {
            setLoading(null);
        }
    };

    const toggleModelVisibility = (providerKey: string, modelId: string) => {
        const p = providers[providerKey];
        if (!p) return;

        const newModels = p.models.map(m =>
            m.id === modelId ? { ...m, visible: !m.visible } : m
        );
        updateProvider(providerKey, { models: newModels });
    }

    const toggleAllModels = (providerKey: string, visible: boolean) => {
        const p = providers[providerKey];
        if (!p) return;

        const newModels = p.models.map(m => ({ ...m, visible }));
        updateProvider(providerKey, { models: newModels });
    };

    const deleteProvider = (key: string) => {
        if (confirm(`Are you sure you want to delete provider '${key}'?`)) {
            const newProviders = { ...providers };
            delete newProviders[key];
            onChange(newProviders);
        }
    };

    const addProvider = () => {
        const id = prompt("Enter a unique ID for the new provider (e.g., 'local-vllm' or 'deepseek'):");
        if (!id) return;
        if (providers[id]) {
            alert("Provider ID already exists.");
            return;
        }

        const newProviders = { ...providers };
        newProviders[id] = {
            enabled: true,
            provider_type: "OpenAICompatible",
            base_url: "http://localhost:1234/v1",
            api_key: "",
            models: []
        };
        onChange(newProviders);
    };

    return (
        <div className="space-y-8">
            <div className="space-y-6">
                {Object.entries(providers).map(([key, config]) => (
                    <div key={key} className="bg-gray-900 border border-gray-800 rounded-xl p-4 transition-all hover:border-gray-700">
                        <div className="flex justify-between items-start mb-4">
                            <div className="flex items-center gap-3">
                                <div className={`w-3 h-3 rounded-full ${config.enabled ? "bg-green-500 shadow-lg shadow-green-500/50" : "bg-gray-600"}`} />
                                <h3 className="font-bold text-lg capitalize">{key}</h3>
                                <span className="text-xs px-2 py-0.5 rounded bg-gray-800 text-gray-400 border border-gray-700">
                                    {config.provider_type}
                                </span>
                            </div>
                            <div className="flex items-center gap-2">
                                <button
                                    onClick={() => updateProvider(key, { enabled: !config.enabled })}
                                    className={`p-2 rounded-lg transition-colors ${config.enabled ? "bg-green-900/30 text-green-400 hover:bg-green-900/50" : "bg-gray-800 text-gray-500 hover:bg-gray-700"}`}
                                    title={config.enabled ? "Disable Provider" : "Enable Provider"}
                                >
                                    {config.enabled ? <CheckCircle size={18} /> : <AlertCircle size={18} />}
                                </button>
                                <button
                                    onClick={() => deleteProvider(key)}
                                    className="p-2 rounded-lg bg-gray-800 text-gray-500 hover:bg-red-900/30 hover:text-red-400 transition-colors"
                                    title="Delete Provider"
                                >
                                    <Trash2 size={18} />
                                </button>
                            </div>
                        </div>

                        <div className="space-y-4">
                            {/* API Key / URL */}
                            {config.provider_type !== "Ollama" && (
                                <div className="space-y-1">
                                    <label className="text-xs font-semibold text-gray-500 uppercase">API Key</label>
                                    <div className="relative">
                                        <input
                                            type="password"
                                            value={config.api_key || ""}
                                            onChange={(e) => updateProvider(key, { api_key: e.target.value })}
                                            className="w-full bg-gray-950 border border-gray-800 rounded-lg px-3 py-2 text-sm text-white focus:ring-1 focus:ring-blue-500 outline-none"
                                            placeholder="sk-..."
                                        />
                                    </div>
                                </div>
                            )}

                            {(config.provider_type === "Ollama" || config.provider_type === "OpenAICompatible") && (
                                <div className="space-y-1">
                                    <label className="text-xs font-semibold text-gray-500 uppercase">Base URL</label>
                                    <input
                                        type="text"
                                        value={config.base_url || ""}
                                        onChange={(e) => updateProvider(key, { base_url: e.target.value })}
                                        className="w-full bg-gray-950 border border-gray-800 rounded-lg px-3 py-2 text-sm text-white focus:ring-1 focus:ring-blue-500 outline-none"
                                        placeholder="http://localhost:11434"
                                    />
                                </div>
                            )}

                            {/* Models */}
                            <div className="pt-2 border-t border-gray-800">
                                <div className="flex justify-between items-center mb-2">
                                    <div className="flex items-center gap-3">
                                        <label className="text-xs font-semibold text-gray-500 uppercase">
                                            Models ({config.models.filter(m => m.visible !== false).length}/{config.models.length})
                                        </label>
                                        {config.models.length > 0 && (
                                            <div className="flex gap-1">
                                                <button
                                                    onClick={() => toggleAllModels(key, true)}
                                                    className="text-[10px] px-1.5 py-0.5 bg-gray-800 hover:bg-gray-700 text-gray-400 rounded uppercase font-bold transition-colors"
                                                    title="Enable All"
                                                >
                                                    All
                                                </button>
                                                <button
                                                    onClick={() => toggleAllModels(key, false)}
                                                    className="text-[10px] px-1.5 py-0.5 bg-gray-800 hover:bg-gray-700 text-gray-400 rounded uppercase font-bold transition-colors"
                                                    title="Disable All"
                                                >
                                                    None
                                                </button>
                                            </div>
                                        )}
                                    </div>
                                    <button
                                        onClick={() => fetchModels(key)}
                                        disabled={loading === key || !config.enabled}
                                        className="flex items-center gap-1.5 px-3 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 disabled:cursor-not-allowed rounded-lg text-xs font-bold text-white transition-colors"
                                    >
                                        {loading === key ? <RefreshCw size={12} className="animate-spin" /> : <RefreshCw size={12} />}
                                        Fetch Models
                                    </button>
                                </div>

                                {config.models.length > 0 ? (
                                    <div className="grid grid-cols-1 md:grid-cols-2 gap-2 max-h-60 overflow-y-auto pr-2 custom-scrollbar">
                                        {config.models.map(m => (
                                            <div
                                                key={m.id}
                                                className={`flex items-center justify-between bg-gray-950 border rounded px-3 py-2 text-xs transition-colors ${m.visible !== false ? "border-gray-800 text-gray-200" : "border-gray-800/50 text-gray-600"}`}
                                            >
                                                <span className="truncate mr-2" title={m.name}>{m.name}</span>
                                                <button
                                                    onClick={() => toggleModelVisibility(key, m.id)}
                                                    className={`p-1 rounded hover:bg-gray-800 ${m.visible !== false ? "text-blue-400" : "text-gray-600"}`}
                                                >
                                                    {m.visible !== false ? <Eye size={14} /> : <EyeOff size={14} />}
                                                </button>
                                            </div>
                                        ))}
                                    </div>
                                ) : (
                                    <div className="text-xs text-gray-600 italic text-center py-2">
                                        No models found. Click fetch to discover.
                                    </div>
                                )}
                            </div>
                        </div>
                    </div>
                ))}
            </div>

            <button
                onClick={addProvider}
                className="w-full py-3 border-2 border-dashed border-gray-800 rounded-xl text-gray-500 hover:border-gray-600 hover:text-gray-300 transition-colors flex items-center justify-center gap-2 font-semibold"
            >
                <div className="w-5 h-5 rounded-full border border-current flex items-center justify-center">+</div>
                Add Custom Provider
            </button>
        </div>
    );
}
