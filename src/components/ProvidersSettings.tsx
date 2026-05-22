
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Eye, EyeOff, RefreshCw, Trash2, CheckCircle, AlertCircle, Search, X, Settings, Save } from "lucide-react";
import { getProviderIcon } from "../utils/providerIcons";

export interface ModelConfig {
    id: string;
    name: string;
    visible?: boolean;
    context_window?: number;
    max_tokens?: number;
    prompt_cost?: string;
    completion_cost?: string;
    parameters?: number;
    description?: string;
    supports_reasoning_effort?: boolean;
    supports_thinking_mode?: boolean;
    supports_extended_thinking?: boolean;
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
    const [editingModelId, setEditingModelId] = useState<string | null>(null);
    const [editContext, setEditContext] = useState<string>("");
    const [editReasoningEffort, setEditReasoningEffort] = useState<boolean | undefined>(undefined);
    const [editThinkingMode, setEditThinkingMode] = useState<boolean | undefined>(undefined);
    const [editExtendedThinking, setEditExtendedThinking] = useState<boolean | undefined>(undefined);

    const startEditing = (m: ModelConfig) => {
        setEditingModelId(m.id);
        setEditContext(m.context_window?.toString() || "");
        setEditReasoningEffort(m.supports_reasoning_effort);
        setEditThinkingMode(m.supports_thinking_mode);
        setEditExtendedThinking(m.supports_extended_thinking);
    };

    const saveEdit = () => {
        if (!editingModelId) return;
        // Parse int, allow empty to unset
        let val: number | undefined = undefined;
        if (editContext.trim()) {
            const parsed = parseInt(editContext.replace(/[^0-9]/g, ""));
            if (!isNaN(parsed)) val = parsed;
        }

        const newModels = config.models.map(m =>
            m.id === editingModelId ? { 
                ...m, 
                context_window: val,
                supports_reasoning_effort: editReasoningEffort,
                supports_thinking_mode: editThinkingMode,
                supports_extended_thinking: editExtendedThinking,
            } : m
        );
        onUpdate({ models: newModels });
        setEditingModelId(null);
    };

    const cancelEdit = () => {
        setEditingModelId(null);
        setEditContext("");
    };

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
        <div className="bg-[var(--color-bg-secondary)] border border-[var(--color-border-primary)] rounded-xl p-4 transition-all hover:border-[var(--color-border-secondary)]">
            <div className="flex justify-between items-start mb-4">
                <div className="flex items-center gap-3">
                    <div className={`text-2xl`} title={config.enabled ? "Enabled" : "Disabled"}>
                        {getProviderIcon(config.provider_type, providerKey)}
                    </div>
                    <div>
                        <h3 className={`font-bold text-lg capitalize ${!config.enabled && "text-[var(--color-text-tertiary)] line-through decoration-[var(--color-text-tertiary)]"}`}>{providerKey}</h3>
                    </div>
                    <span className="text-xs px-2 py-0.5 rounded bg-[var(--color-bg-tertiary)] text-[var(--color-text-secondary)] border border-[var(--color-border-secondary)]">
                        {config.provider_type}
                    </span>
                </div>
                <div className="flex items-center gap-2">
                    <button
                        onClick={() => onUpdate({ enabled: !config.enabled })}
                        className={`p-2 rounded-lg transition-colors ${config.enabled ? "bg-green-900/30 text-green-400 hover:bg-green-900/50" : "bg-[var(--color-bg-tertiary)] text-[var(--color-text-tertiary)] hover:bg-[var(--color-hover-bg)]"}`}
                        title={config.enabled ? "Disable Provider" : "Enable Provider"}
                    >
                        {config.enabled ? <CheckCircle size={18} /> : <AlertCircle size={18} />}
                    </button>
                    <button
                        onClick={onDelete}
                        className="p-2 rounded-lg bg-[var(--color-bg-tertiary)] text-[var(--color-text-tertiary)] hover:bg-red-900/30 hover:text-red-400 transition-colors"
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
                        <label className="text-xs font-semibold text-[var(--color-text-tertiary)] uppercase">API Key</label>
                        <div className="relative">
                            <input
                                type="password"
                                value={config.api_key || ""}
                                onChange={(e) => onUpdate({ api_key: e.target.value })}
                                className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-primary)] rounded-lg px-3 py-2 text-sm text-[var(--color-text-primary)] focus:ring-1 focus:ring-blue-500 outline-none"
                                placeholder="sk-..."
                            />
                        </div>
                    </div>
                )}

                {(config.provider_type === "Ollama" || config.provider_type === "OpenAICompatible" || config.provider_type === "Anthropic") && (
                    <div className="space-y-1">
                        <label className="text-xs font-semibold text-[var(--color-text-tertiary)] uppercase">
                            Base URL{config.provider_type === "Anthropic" ? " (optional)" : ""}
                        </label>
                        <input
                            type="text"
                            value={config.base_url || ""}
                            onChange={(e) => onUpdate({ base_url: e.target.value })}
                            className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-primary)] rounded-lg px-3 py-2 text-sm text-[var(--color-text-primary)] focus:ring-1 focus:ring-blue-500 outline-none"
                            placeholder={
                                config.provider_type === "Ollama"
                                    ? "http://localhost:11434"
                                    : config.provider_type === "Anthropic"
                                        ? "https://api.anthropic.com"
                                        : "http://localhost:1234/v1"
                            }
                        />
                        {config.provider_type === "Anthropic" && (
                            <p className="text-[10px] text-[var(--color-text-tertiary)]">
                                Leave blank for the official Anthropic API. Set to any Anthropic-compatible endpoint (e.g. a self-hosted proxy or third-party provider).
                            </p>
                        )}
                    </div>
                )}

                {/* Models */}
                <div className="pt-2 border-t border-[var(--color-border-primary)]">
                    <div className="flex flex-col gap-2 mb-2">
                        <div className="flex justify-between items-center">
                            <label className="text-xs font-semibold text-[var(--color-text-tertiary)] uppercase">
                                Models ({config.models.filter(m => m.visible !== false).length}/{config.models.length})
                            </label>

                            <button
                                onClick={onFetch}
                                disabled={loading || !config.enabled}
                                className="flex items-center gap-1.5 px-3 py-1.5 btn-primary rounded-lg text-xs font-bold transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
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
                                    <Search size={14} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--color-text-tertiary)]" />
                                    <input
                                        type="text"
                                        placeholder="Filter models..."
                                        value={searchQuery}
                                        onChange={(e) => setSearchQuery(e.target.value)}
                                        className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-primary)] rounded-lg pl-8 pr-8 py-1.5 text-xs text-[var(--color-text-primary)] focus:ring-1 focus:ring-blue-500 outline-none"
                                    />
                                    {searchQuery && (
                                        <button
                                            onClick={() => setSearchQuery("")}
                                            className="absolute right-2 top-1/2 -translate-y-1/2 text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)]"
                                        >
                                            <X size={12} />
                                        </button>
                                    )}
                                </div>

                                {/* Bulk Actions */}
                                <div className="flex gap-1">
                                    <button
                                        onClick={() => toggleAllModels(true)}
                                        className="text-[10px] px-2 py-1.5 bg-[var(--color-bg-tertiary)] hover:bg-[var(--color-hover-bg)] text-[var(--color-text-secondary)] rounded uppercase font-bold transition-colors"
                                        title="Enable All"
                                    >
                                        All
                                    </button>
                                    <button
                                        onClick={() => toggleAllModels(false)}
                                        className="text-[10px] px-2 py-1.5 bg-[var(--color-bg-tertiary)] hover:bg-[var(--color-hover-bg)] text-[var(--color-text-secondary)] rounded uppercase font-bold transition-colors"
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
                                        className={`flex items-center justify-between bg-[var(--color-bg-primary)] border rounded px-3 py-2 text-xs transition-colors ${m.visible !== false ? "border-[var(--color-border-primary)] text-[var(--color-text-primary)]" : "border-[var(--color-border-primary)]/50 text-[var(--color-text-tertiary)]"}`}
                                    >
                                        {editingModelId === m.id ? (
                                            <div className="flex flex-col gap-2 w-full">
                                                <div className="flex items-center gap-2 w-full">
                                                    <span className="truncate flex-1 font-mono text-[var(--color-text-secondary)]" title={m.name}>{m.name}</span>
                                                    <input
                                                        className="w-20 bg-[var(--color-bg-tertiary)] border border-[var(--color-border-secondary)] rounded px-1 py-0.5 text-right focus:border-blue-500 outline-none"
                                                        placeholder="Context"
                                                        value={editContext}
                                                        onChange={e => setEditContext(e.target.value)}
                                                        autoFocus
                                                        onKeyDown={e => {
                                                            if (e.key === 'Enter') saveEdit();
                                                            if (e.key === 'Escape') cancelEdit();
                                                        }}
                                                    />
                                                    <button onClick={saveEdit} className="p-1 hover:text-green-500 text-[var(--color-text-secondary)]"><Save size={14} /></button>
                                                    <button onClick={cancelEdit} className="p-1 hover:text-red-500 text-[var(--color-text-secondary)]"><X size={14} /></button>
                                                </div>
                                                <div className="flex items-center gap-3 text-[10px] text-[var(--color-text-tertiary)]">
                                                    <label className="flex items-center gap-1 cursor-pointer" title="OpenAI o1/o3 style reasoning_effort parameter">
                                                        <input
                                                            type="checkbox"
                                                            checked={editReasoningEffort ?? false}
                                                            onChange={e => setEditReasoningEffort(e.target.checked)}
                                                            className="rounded"
                                                        />
                                                        <span>Effort</span>
                                                    </label>
                                                    <label className="flex items-center gap-1 cursor-pointer" title="DeepSeek style thinking mode">
                                                        <input
                                                            type="checkbox"
                                                            checked={editThinkingMode ?? false}
                                                            onChange={e => setEditThinkingMode(e.target.checked)}
                                                            className="rounded"
                                                        />
                                                        <span>Thinking</span>
                                                    </label>
                                                    <label className="flex items-center gap-1 cursor-pointer" title="Anthropic Claude 4 extended thinking">
                                                        <input
                                                            type="checkbox"
                                                            checked={editExtendedThinking ?? false}
                                                            onChange={e => setEditExtendedThinking(e.target.checked)}
                                                            className="rounded"
                                                        />
                                                        <span>Extended</span>
                                                    </label>
                                                </div>
                                            </div>
                                        ) : (
                                            <>
                                                <div className="flex items-center gap-2 overflow-hidden flex-1">
                                                    <span className="truncate" title={m.name}>{m.name}</span>
                                                    {m.context_window && (
                                                        <span className="px-1.5 py-0.5 rounded-full bg-[var(--color-bg-tertiary)] text-[var(--color-text-tertiary)] text-[10px]">
                                                            {Math.round(m.context_window / 1000)}k
                                                        </span>
                                                    )}
                                                    {(m.supports_reasoning_effort || m.supports_thinking_mode || m.supports_extended_thinking) && (
                                                        <span className="px-1.5 py-0.5 rounded-full bg-purple-900/30 text-purple-300 text-[10px]" title="Supports reasoning features">
                                                            💭
                                                        </span>
                                                    )}
                                                </div>
                                                <div className="flex items-center gap-1">
                                                    <button
                                                        onClick={() => startEditing(m)}
                                                        className="p-1 rounded hover:bg-[var(--color-bg-tertiary)] text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)]"
                                                        title="Edit Context Window"
                                                    >
                                                        <Settings size={14} />
                                                    </button>
                                                    <button
                                                        onClick={() => toggleModelVisibility(m.id)}
                                                        className={`p-1 rounded hover:bg-[var(--color-bg-tertiary)] ${m.visible !== false ? "text-[var(--color-accent-primary)]" : "text-[var(--color-text-tertiary)]"}`}
                                                        title={m.visible !== false ? "Hide Model" : "Show Model"}
                                                    >
                                                        {m.visible !== false ? <Eye size={14} /> : <EyeOff size={14} />}
                                                    </button>
                                                </div>
                                            </>
                                        )}
                                    </div>
                                ))
                            ) : (
                                <div className="col-span-2 text-xs text-[var(--color-text-tertiary)] italic text-center py-4">
                                    No models match "{searchQuery}"
                                </div>
                            )}
                        </div>
                    ) : (
                        <div className="text-xs text-[var(--color-text-tertiary)] italic text-center py-2">
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
        // Only set base_url for providers that need it. OpenAI uses a fixed cloud
        // endpoint. Anthropic accepts an optional override for Anthropic-compatible
        // endpoints; pass it through only when the user actually filled it in.
        const trimmedBaseUrl = newBaseUrl.trim();
        const baseUrlForType =
            newType === "Ollama"
                ? (trimmedBaseUrl || "http://localhost:11434")
                : newType === "OpenAICompatible"
                    ? trimmedBaseUrl
                    : newType === "Anthropic"
                        ? (trimmedBaseUrl || undefined)
                        : undefined; // OpenAI uses fixed cloud endpoint

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
                className="w-full py-3 border-2 border-dashed border-[var(--color-border-primary)] rounded-xl text-[var(--color-text-tertiary)] hover:border-[var(--color-border-secondary)] hover:text-[var(--color-text-secondary)] transition-colors flex items-center justify-center gap-2 font-semibold"
            >
                <div className="w-5 h-5 rounded-full border border-current flex items-center justify-center">+</div>
                Add Custom Provider
            </button>

            {isAddOpen && (
                <div className="fixed inset-0 bg-black/70 z-50 flex items-center justify-center p-4">
                    <div className="bg-[var(--color-bg-secondary)] border border-[var(--color-border-primary)] rounded-xl w-full max-w-lg overflow-hidden">
                        <div className="p-4 border-b border-[var(--color-border-primary)] flex items-center justify-between">
                            <h4 className="font-bold">Add Model Provider</h4>
                            <button className="text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]" onClick={() => setIsAddOpen(false)}>×</button>
                        </div>
                        <div className="p-4 space-y-4">
                            <div>
                                <label className="block text-xs font-semibold text-[var(--color-text-tertiary)] uppercase mb-1">Provider ID</label>
                                <input
                                    value={newId}
                                    onChange={(e) => setNewId(e.target.value)}
                                    placeholder="e.g. local-vllm or deepseek"
                                    className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-primary)] rounded-lg px-3 py-2 text-sm text-[var(--color-text-primary)] focus:ring-1 focus:ring-blue-500 outline-none"
                                />
                            </div>
                            <div>
                                <label className="block text-xs font-semibold text-[var(--color-text-tertiary)] uppercase mb-1">Provider Type</label>
                                <select
                                    value={newType}
                                    onChange={(e) => {
                                        const next = e.target.value as ProviderType;
                                        setNewType(next);
                                        // Reset base URL placeholder/default per provider type so the
                                        // user doesn't carry over a stale localhost:1234/v1 when they
                                        // switch to Anthropic.
                                        if (next === "Ollama") {
                                            setNewBaseUrl("http://localhost:11434");
                                        } else if (next === "OpenAICompatible") {
                                            setNewBaseUrl("http://localhost:1234/v1");
                                        } else {
                                            setNewBaseUrl("");
                                        }
                                    }}
                                    className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-primary)] rounded-lg px-3 py-2 text-sm text-[var(--color-text-primary)] focus:ring-1 focus:ring-blue-500 outline-none"
                                >
                                    <option value="OpenAI">OpenAI</option>
                                    <option value="Anthropic">Anthropic</option>
                                    <option value="Ollama">Ollama</option>
                                    <option value="OpenAICompatible">OpenAICompatible</option>
                                </select>
                            </div>
                            {(newType === "Ollama" || newType === "OpenAICompatible" || newType === "Anthropic") && (
                                <div>
                                    <label className="block text-xs font-semibold text-[var(--color-text-tertiary)] uppercase mb-1">
                                        Base URL{newType === "Anthropic" ? " (optional)" : ""}
                                    </label>
                                    <input
                                        value={newBaseUrl}
                                        onChange={(e) => setNewBaseUrl(e.target.value)}
                                        placeholder={
                                            newType === "Ollama"
                                                ? "http://localhost:11434"
                                                : newType === "Anthropic"
                                                    ? "https://api.anthropic.com"
                                                    : "http://localhost:1234/v1"
                                        }
                                        className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-primary)] rounded-lg px-3 py-2 text-sm text-[var(--color-text-primary)] focus:ring-1 focus:ring-blue-500 outline-none"
                                    />
                                    {newType === "Anthropic" && (
                                        <p className="mt-1 text-[10px] text-[var(--color-text-tertiary)]">
                                            Leave blank for the official Anthropic API. Override only for Anthropic-compatible endpoints.
                                        </p>
                                    )}
                                </div>
                            )}
                            {newType !== "Ollama" && (
                                <div>
                                    <label className="block text-xs font-semibold text-[var(--color-text-tertiary)] uppercase mb-1">API Key</label>
                                    <input
                                        type="password"
                                        value={newApiKey}
                                        onChange={(e) => setNewApiKey(e.target.value)}
                                        placeholder="sk-..."
                                        className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-primary)] rounded-lg px-3 py-2 text-sm text-[var(--color-text-primary)] focus:ring-1 focus:ring-blue-500 outline-none"
                                    />
                                </div>
                            )}
                        </div>
                        <div className="p-4 border-t border-[var(--color-border-primary)] flex justify-end gap-2">
                            <button className="px-4 py-2 rounded-lg hover:bg-[var(--color-bg-tertiary)] text-[var(--color-text-secondary)] font-bold text-sm" onClick={() => setIsAddOpen(false)}>Cancel</button>
                            <button className="px-6 py-2 rounded-lg btn-primary font-bold text-sm" onClick={confirmAddProvider}>Add</button>
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
}
