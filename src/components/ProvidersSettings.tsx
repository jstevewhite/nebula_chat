
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Eye, EyeOff, RefreshCw, Trash2, CheckCircle, AlertCircle, Search, X } from "lucide-react";
import { getProviderIcon } from "../utils/providerIcons";

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

interface ProviderCardProps {
    providerKey: string;
    config: ProviderConfig;
    onUpdate: (updates: Partial<ProviderConfig>) => void;
    onDelete: () => void;
    onFetch: () => void;
    loading: boolean;
}

function ProviderCard({ providerKey, config, onUpdate, onDelete, onFetch, loading }: ProviderCardProps) {
    const [searchQuery, setSearchQuery] = useState("");

    const toggleModelVisibility = (modelId: string) => {
        const newModels = config.models.map(m =>
            m.id === modelId ? { ...m, visible: !m.visible } : m
        );
        onUpdate({ models: newModels });
    }

    const toggleAllModels = (visible: boolean) => {
        const newModels = config.models.map(m => ({ ...m, visible }));
        onUpdate({ models: newModels });
    };

    const filteredModels = config.models.filter(m =>
        m.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
        m.id.toLowerCase().includes(searchQuery.toLowerCase())
    );

    return (
        <div className="bg-gray-900 border border-gray-800 rounded-xl p-4 transition-all hover:border-gray-700">
            <div className="flex justify-between items-start mb-4">
                <div className="flex items-center gap-3">
                    <div className={`text-2xl`} title={config.enabled ? "Enabled" : "Disabled"}>
                        {getProviderIcon(config.provider_type, providerKey)}
                    </div>
                    <div>
                        <h3 className={`font-bold text-lg capitalize ${!config.enabled && "text-gray-500 line-through decoration-gray-500"}`}>{providerKey}</h3>
                    </div>
                    <span className="text-xs px-2 py-0.5 rounded bg-gray-800 text-gray-400 border border-gray-700">
                        {config.provider_type}
                    </span>
                </div>
                <div className="flex items-center gap-2">
                    <button
                        onClick={() => onUpdate({ enabled: !config.enabled })}
                        className={`p-2 rounded-lg transition-colors ${config.enabled ? "bg-green-900/30 text-green-400 hover:bg-green-900/50" : "bg-gray-800 text-gray-500 hover:bg-gray-700"}`}
                        title={config.enabled ? "Disable Provider" : "Enable Provider"}
                    >
                        {config.enabled ? <CheckCircle size={18} /> : <AlertCircle size={18} />}
                    </button>
                    <button
                        onClick={onDelete}
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
                                onChange={(e) => onUpdate({ api_key: e.target.value })}
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
                            onChange={(e) => onUpdate({ base_url: e.target.value })}
                            className="w-full bg-gray-950 border border-gray-800 rounded-lg px-3 py-2 text-sm text-white focus:ring-1 focus:ring-blue-500 outline-none"
                            placeholder="http://localhost:11434"
                        />
                    </div>
                )}

                {/* Models */}
                <div className="pt-2 border-t border-gray-800">
                    <div className="flex flex-col gap-2 mb-2">
                        <div className="flex justify-between items-center">
                            <label className="text-xs font-semibold text-gray-500 uppercase">
                                Models ({config.models.filter(m => m.visible !== false).length}/{config.models.length})
                            </label>

                            <button
                                onClick={onFetch}
                                disabled={loading || !config.enabled}
                                className="flex items-center gap-1.5 px-3 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 disabled:cursor-not-allowed rounded-lg text-xs font-bold text-white transition-colors"
                            >
                                {loading ? <RefreshCw size={12} className="animate-spin" /> : <RefreshCw size={12} />}
                                Fetch Models
                            </button>
                        </div>

                        {/* Controls Row: Search + Bulk Actions */}
                        {config.models.length > 0 && (
                            <div className="flex gap-2 items-center">
                                {/* Search Input */}
                                <div className="relative flex-1">
                                    <Search size={14} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-500" />
                                    <input
                                        type="text"
                                        placeholder="Filter models..."
                                        value={searchQuery}
                                        onChange={(e) => setSearchQuery(e.target.value)}
                                        className="w-full bg-gray-950 border border-gray-800 rounded-lg pl-8 pr-8 py-1.5 text-xs text-white focus:ring-1 focus:ring-blue-500 outline-none"
                                    />
                                    {searchQuery && (
                                        <button
                                            onClick={() => setSearchQuery("")}
                                            className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-500 hover:text-white"
                                        >
                                            <X size={12} />
                                        </button>
                                    )}
                                </div>

                                {/* Bulk Actions */}
                                <div className="flex gap-1">
                                    <button
                                        onClick={() => toggleAllModels(true)}
                                        className="text-[10px] px-2 py-1.5 bg-gray-800 hover:bg-gray-700 text-gray-400 rounded uppercase font-bold transition-colors"
                                        title="Enable All"
                                    >
                                        All
                                    </button>
                                    <button
                                        onClick={() => toggleAllModels(false)}
                                        className="text-[10px] px-2 py-1.5 bg-gray-800 hover:bg-gray-700 text-gray-400 rounded uppercase font-bold transition-colors"
                                        title="Disable All"
                                    >
                                        None
                                    </button>
                                </div>
                            </div>
                        )}
                    </div>

                    {config.models.length > 0 ? (
                        <div className="grid grid-cols-1 md:grid-cols-2 gap-2 max-h-60 overflow-y-auto pr-2 custom-scrollbar">
                            {filteredModels.length > 0 ? (
                                filteredModels.map(m => (
                                    <div
                                        key={m.id}
                                        className={`flex items-center justify-between bg-gray-950 border rounded px-3 py-2 text-xs transition-colors ${m.visible !== false ? "border-gray-800 text-gray-200" : "border-gray-800/50 text-gray-600"}`}
                                    >
                                        <span className="truncate mr-2" title={m.name}>{m.name}</span>
                                        <button
                                            onClick={() => toggleModelVisibility(m.id)}
                                            className={`p-1 rounded hover:bg-gray-800 ${m.visible !== false ? "text-blue-400" : "text-gray-600"}`}
                                        >
                                            {m.visible !== false ? <Eye size={14} /> : <EyeOff size={14} />}
                                        </button>
                                    </div>
                                ))
                            ) : (
                                <div className="col-span-2 text-xs text-gray-600 italic text-center py-4">
                                    No models match "{searchQuery}"
                                </div>
                            )}
                        </div>
                    ) : (
                        <div className="text-xs text-gray-600 italic text-center py-2">
                            No models found. Click fetch to discover.
                        </div>
                    )}
                </div>
            </div>
        </div>
    );
}

interface ProvidersSettingsProps {
    providers: Record<string, ProviderConfig>;
    onChange: (providers: Record<string, ProviderConfig>) => void;
}

export default function ProvidersSettings({ providers, onChange }: ProvidersSettingsProps) {
    const [loading, setLoading] = useState<string | null>(null);

    // Add Provider Modal state
    const [isAddOpen, setIsAddOpen] = useState(false);
    const [newId, setNewId] = useState("");
    const [newType, setNewType] = useState<ProviderType>("OpenAICompatible");
    const [newBaseUrl, setNewBaseUrl] = useState("http://localhost:1234/v1");
    const [newApiKey, setNewApiKey] = useState("");

    const resetAddForm = () => {
        setNewId("");
        setNewType("OpenAICompatible");
        setNewBaseUrl("http://localhost:1234/v1");
        setNewApiKey("");
    };

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

    const deleteProvider = (key: string) => {
        if (confirm(`Are you sure you want to delete provider '${key}'?`)) {
            const newProviders = { ...providers };
            delete newProviders[key];
            onChange(newProviders);
        }
    };

    const openAddProvider = () => {
        resetAddForm();
        setIsAddOpen(true);
    };

    const confirmAddProvider = () => {
        const id = newId.trim();
        if (!id) {
            alert("Please enter a provider ID.");
            return;
        }
        if (providers[id]) {
            alert("Provider ID already exists.");
            return;
        }

        const newProviders = { ...providers };
        // Only set base_url for providers that require it (Ollama/OpenAICompatible)
        const baseUrlForType =
            newType === "Ollama"
                ? (newBaseUrl || "http://localhost:11434")
                : newType === "OpenAICompatible"
                    ? newBaseUrl
                    : undefined; // OpenAI / Anthropic use fixed cloud endpoints

        const apiKeyForType = newType === "Ollama" ? "" : newApiKey;

        newProviders[id] = {
            enabled: true,
            provider_type: newType,
            base_url: baseUrlForType,
            api_key: apiKeyForType,
            models: []
        };
        onChange(newProviders);
        setIsAddOpen(false);
    };

    return (
        <div className="space-y-8">
            <div className="space-y-6">
                {Object.entries(providers).map(([key, config]) => (
                    <ProviderCard
                        key={key}
                        providerKey={key}
                        config={config}
                        onUpdate={(updates) => updateProvider(key, updates)}
                        onDelete={() => deleteProvider(key)}
                        onFetch={() => fetchModels(key)}
                        loading={loading === key}
                    />
                ))}
            </div>

            <button
                onClick={openAddProvider}
                className="w-full py-3 border-2 border-dashed border-gray-800 rounded-xl text-gray-500 hover:border-gray-600 hover:text-gray-300 transition-colors flex items-center justify-center gap-2 font-semibold"
            >
                <div className="w-5 h-5 rounded-full border border-current flex items-center justify-center">+</div>
                Add Custom Provider
            </button>

            {isAddOpen && (
                <div className="fixed inset-0 bg-black/70 z-50 flex items-center justify-center p-4">
                    <div className="bg-gray-900 border border-gray-800 rounded-xl w-full max-w-lg overflow-hidden">
                        <div className="p-4 border-b border-gray-800 flex items-center justify-between">
                            <h4 className="font-bold">Add Model Provider</h4>
                            <button className="text-gray-400 hover:text-white" onClick={() => setIsAddOpen(false)}>×</button>
                        </div>
                        <div className="p-4 space-y-4">
                            <div>
                                <label className="block text-xs font-semibold text-gray-500 uppercase mb-1">Provider ID</label>
                                <input
                                    value={newId}
                                    onChange={(e) => setNewId(e.target.value)}
                                    placeholder="e.g. local-vllm or deepseek"
                                    className="w-full bg-gray-950 border border-gray-800 rounded-lg px-3 py-2 text-sm text-white focus:ring-1 focus:ring-blue-500 outline-none"
                                />
                            </div>
                            <div>
                                <label className="block text-xs font-semibold text-gray-500 uppercase mb-1">Provider Type</label>
                                <select
                                    value={newType}
                                    onChange={(e) => setNewType(e.target.value as ProviderType)}
                                    className="w-full bg-gray-950 border border-gray-800 rounded-lg px-3 py-2 text-sm text-white focus:ring-1 focus:ring-blue-500 outline-none"
                                >
                                    <option value="OpenAI">OpenAI</option>
                                    <option value="Anthropic">Anthropic</option>
                                    <option value="Ollama">Ollama</option>
                                    <option value="OpenAICompatible">OpenAICompatible</option>
                                </select>
                            </div>
                            {(newType === "Ollama" || newType === "OpenAICompatible") && (
                                <div>
                                    <label className="block text-xs font-semibold text-gray-500 uppercase mb-1">Base URL</label>
                                    <input
                                        value={newBaseUrl}
                                        onChange={(e) => setNewBaseUrl(e.target.value)}
                                        placeholder={newType === "Ollama" ? "http://localhost:11434" : "http://localhost:1234/v1"}
                                        className="w-full bg-gray-950 border border-gray-800 rounded-lg px-3 py-2 text-sm text-white focus:ring-1 focus:ring-blue-500 outline-none"
                                    />
                                </div>
                            )}
                            {newType !== "Ollama" && (
                                <div>
                                    <label className="block text-xs font-semibold text-gray-500 uppercase mb-1">API Key</label>
                                    <input
                                        type="password"
                                        value={newApiKey}
                                        onChange={(e) => setNewApiKey(e.target.value)}
                                        placeholder="sk-..."
                                        className="w-full bg-gray-950 border border-gray-800 rounded-lg px-3 py-2 text-sm text-white focus:ring-1 focus:ring-blue-500 outline-none"
                                    />
                                </div>
                            )}
                        </div>
                        <div className="p-4 border-t border-gray-800 flex justify-end gap-2">
                            <button className="px-4 py-2 rounded-lg hover:bg-gray-800 text-gray-400 font-bold text-sm" onClick={() => setIsAddOpen(false)}>Cancel</button>
                            <button className="px-6 py-2 rounded-lg bg-blue-600 hover:bg-blue-500 text-white font-bold text-sm" onClick={confirmAddProvider}>Add</button>
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
}
