import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Server, Plus, Edit2, Book, Trash2, Palette, Brain, RefreshCw } from "lucide-react";
import ProvidersSettings, { ProviderConfig } from "./ProvidersSettings";
import PromptsSettings from "./PromptsSettings";
import { ThemeSelector } from "./ThemeSelector";
import { CustomSelect } from "./ui/CustomSelect";
import { useTheme } from "../contexts/ThemeContext";

const AVAILABLE_FONTS = [
    { id: "Inter", label: "Inter (Default)", value: "Inter" },
    { id: "system-ui", label: "System UI", value: "system-ui, -apple-system, sans-serif" },
    { id: "Roboto", label: "Roboto", value: "Roboto" },
    { id: "Open Sans", label: "Open Sans", value: "Open Sans" },
    { id: "Lato", label: "Lato", value: "Lato" },
    { id: "Montserrat", label: "Montserrat", value: "Montserrat" },
    { id: "Fira Code", label: "Fira Code (Monospace)", value: "Fira Code, monospace" },
    { id: "JetBrains Mono", label: "JetBrains Mono (Monospace)", value: "JetBrains Mono, monospace" },
];

const FONT_SIZES = [
    { id: "12", label: "12px", value: "12" },
    { id: "13", label: "13px", value: "13" },
    { id: "14", label: "14px", value: "14" },
    { id: "15", label: "15px", value: "15" },
    { id: "16", label: "16px", value: "16" },
    { id: "18", label: "18px", value: "18" },
    { id: "20", label: "20px", value: "20" },
];

const FONT_WEIGHTS = [
    { id: "300", label: "Light", value: "300" },
    { id: "400", label: "Normal", value: "400" },
    { id: "500", label: "Medium", value: "500" },
    { id: "600", label: "Semibold", value: "600" },
    { id: "700", label: "Bold", value: "700" },
];

interface Fact {
    id: string;
    subject: string;
    predicate: string;
    object: string;
    object_kind: "entity" | "literal";
    confidence: number;
    source_message_id?: string | null;
    created_at: string;
    updated_at: string;
}

export default function SettingsPage() {
    const { fontSettings, setFontSettings } = useTheme();
    // const colorScheme = theme === 'light' || theme === 'solarized-light' ? 'light' : 'dark'; // Unused

    const [servers, setServers] = useState<{ name: string, status: 'connected' | 'error' | 'unknown', config: any }[]>([]);
    const [providers, setProviders] = useState<Record<string, ProviderConfig>>({});
    const [fullSettings, setFullSettings] = useState<any>({});
    const [status, setStatus] = useState("");

    const [entities, setEntities] = useState<string[]>(["user"]);
    const [selectedEntity, setSelectedEntity] = useState<string>("user");
    const [entityFacts, setEntityFacts] = useState<Fact[]>([]);
    const [factsLoading, setFactsLoading] = useState(false);
    const [factsError, setFactsError] = useState<string | null>(null);

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

    const reloadFactsForEntity = async (
        entityOverride?: string,
        settingsOverride?: any,
    ) => {
        const settings = settingsOverride ?? fullSettings;
        const memoryEnabled = settings.memory_enabled ?? true;
        if (!memoryEnabled) {
            setEntityFacts([]);
            return;
        }

        const targetEntity = entityOverride || selectedEntity || "user";

        try {
            setFactsLoading(true);
            const [rawEntities, facts] = await Promise.all([
                invoke<string[]>("list_fact_entities", { limit: 100 }).catch(() => []),
                targetEntity === "user"
                    ? invoke<Fact[]>("list_user_facts")
                    : invoke<Fact[]>("list_facts_for_entity", { entity: targetEntity, limit: 200 }),
            ]);

            const baseEntities = ["user", ...rawEntities.filter(Boolean)];
            if (!baseEntities.includes(targetEntity)) {
                baseEntities.push(targetEntity);
            }
            const uniqueEntities = Array.from(new Set(baseEntities));
            setEntities(uniqueEntities);
            setEntityFacts(facts);
            setFactsError(null);
        } catch (e) {
            console.error("Failed to load facts/entities", e);
            setFactsError(String(e));
        } finally {
            setFactsLoading(false);
        }
    };

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

            // Load entity list + facts for the currently selected entity when memory is enabled
            await reloadFactsForEntity(undefined, settings);

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
        <div className="p-6 bg-[var(--color-bg-primary)] h-full text-[var(--color-text-primary)] overflow-auto font-sans relative">
            {/* Status Banner */}
            {status && (
                <div className={`mb-6 p-4 rounded-lg flex items-center gap-2 border ${status.includes("Error") || status.includes("Warning") || status.includes("Failed")
                    ? "bg-red-900/20 border-red-500/50 text-red-200"
                    : "bg-blue-900/20 border-blue-500/50 text-blue-200"
                    }`}>
                    <div className="flex-1 font-mono text-sm">{status}</div>
                    <button onClick={() => setStatus("")} className="px-2 hover:bg-[var(--color-hover-bg)] rounded">&times;</button>
                </div>
            )}

            <h2 className="text-2xl font-bold mb-6 flex items-center gap-2 text-[var(--color-text-primary)]">
                <Book className="text-blue-500" /> System Prompts
            </h2>
            <div className="mb-10">
                <PromptsSettings />
            </div>

            <div className="bg-[var(--color-bg-secondary)] p-6 rounded-xl border border-[var(--color-border-primary)] shadow-xl mb-8">
                <h3 className="text-lg font-bold mb-4 flex items-center gap-2">
                    <Book className="w-5 h-5 text-purple-500" /> Intelligence Settings
                </h3>

                <div className="mb-4">
                    <div className="flex items-center justify-between gap-4 mb-3">
                        <div>
                            <label className="block text-sm font-bold text-[var(--color-text-secondary)]">
                                Long-term Memory
                            </label>
                            <p className="text-xs text-[var(--color-text-tertiary)]">
                                Enable retrieval/injection of relevant memories into chats.
                            </p>
                        </div>
                        <label className="flex items-center gap-2 text-sm text-[var(--color-text-secondary)] select-none">
                            <input
                                type="checkbox"
                                checked={fullSettings.memory_enabled ?? true}
                                onChange={(e) => setFullSettings({ ...fullSettings, memory_enabled: e.target.checked })}
                                className="h-4 w-4 rounded border-[var(--color-border-secondary)] bg-[var(--color-bg-primary)]"
                            />
                            Enabled
                        </label>
                    </div>

                    <label className="block text-sm font-bold text-[var(--color-text-secondary)] mb-2">
                        Memory Strategy Model
                    </label>
                    <p className="text-xs text-[var(--color-text-tertiary)] mb-2">
                        Select a model to use for analyzing and summarizing retrieved memories.
                        A smaller model (e.g., Llama 3 8B) is recommended for speed.
                    </p>
                    <CustomSelect
                        disabled={!(fullSettings.memory_enabled ?? true)}
                        value={fullSettings.context_model || ""}
                        onChange={(val) => setFullSettings({ ...fullSettings, context_model: val })}
                        options={[
                            { id: "none", label: "None (Raw Injection)", value: "" },
                            ...Object.entries(providers).flatMap(([pkey, pval]) =>
                                pval.models.filter(m => m.visible).map(m => ({
                                    id: `${pkey}::${m.id}`,
                                    label: `${pkey} - ${m.name}`,
                                    value: `${pkey}::${m.id}`
                                }))
                            )
                        ]}
                        className={!(fullSettings.memory_enabled ?? true) ? "opacity-50" : ""}
                    />

                    <div className="mt-4">
                        <label className="block text-sm font-bold text-[var(--color-text-secondary)] mb-2">
                            Conversation Turns Included
                        </label>
                        <p className="text-xs text-[var(--color-text-tertiary)] mb-2">
                            How many recent turns (user/assistant pairs) to include when deciding which memories are relevant.
                            Set to 0 to disable.
                        </p>
                        <CustomSelect
                            disabled={!(fullSettings.memory_enabled ?? true)}
                            value={String(fullSettings.context_turns ?? 0)}
                            onChange={(val) => setFullSettings({ ...fullSettings, context_turns: Number(val) })}
                            options={[0, 1, 2, 3, 4, 6, 8, 10].map(n => ({
                                id: String(n),
                                label: String(n),
                                value: String(n)
                            }))}
                            className={!(fullSettings.memory_enabled ?? true) ? "opacity-50" : ""}
                        />

                    </div>

                    {/* Fact Memory (Entities) */}
                    <div className="mt-6 border-t border-[var(--color-border-primary)] pt-4">
                        <div className="flex items-center justify-between mb-3 gap-4">
                            <div>
                                <label className="block text-sm font-bold text-[var(--color-text-secondary)]">
                                    Fact Memory (Entities)
                                </label>
                                <p className="text-xs text-[var(--color-text-tertiary)]">
                                    Review and edit durable facts about the selected entity (defaults to you as "user").
                                </p>
                                <div className="flex items-center gap-2 mt-2 text-xs">
                                    <span className="text-[var(--color-text-tertiary)]">Entity:</span>
                                    <div className="w-40">
                                        <CustomSelect
                                            disabled={!(fullSettings.memory_enabled ?? true)}
                                            value={selectedEntity}
                                            onChange={(val) => {
                                                const next = val || "user";
                                                setSelectedEntity(next);
                                                reloadFactsForEntity(next);
                                            }}
                                            options={(entities || []).map((e) => ({
                                                id: e,
                                                label: e === "user" ? "user (you)" : e,
                                                value: e,
                                            }))}
                                            className={!(fullSettings.memory_enabled ?? true) ? "opacity-50" : ""}
                                            filterable
                                            filterPlaceholder="Filter entities..."
                                        />
                                    </div>
                                </div>
                            </div>
                            <button
                                onClick={async () => {
                                    if (!(fullSettings.memory_enabled ?? true)) return;
                                    await reloadFactsForEntity();
                                }}
                                disabled={factsLoading || !(fullSettings.memory_enabled ?? true)}
                                className={`inline-flex items-center gap-1 px-3 py-1.5 rounded-lg border text-xs font-semibold transition-colors
                                    ${(fullSettings.memory_enabled ?? true)
                                        ? "border-[var(--color-border-secondary)] bg-[var(--color-bg-tertiary)] hover:bg-[var(--color-hover-bg)] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]"
                                        : "border-[var(--color-border-secondary)]/40 bg-transparent text-[var(--color-text-tertiary)] cursor-not-allowed"}
                                `}
                            >
                                <RefreshCw className={`w-3 h-3 ${factsLoading ? "animate-spin" : ""}`} />
                                Refresh
                            </button>
                        </div>

                        {factsError && (
                            <div className="mb-2 text-xs text-red-300 bg-red-900/30 border border-red-700/40 rounded-lg p-2">
                                {factsError}
                            </div>
                        )}

                        <div className={`space-y-2 max-h-64 overflow-y-auto custom-scrollbar ${!(fullSettings.memory_enabled ?? true) ? "opacity-50" : ""}`}>
                            {(entityFacts || []).length === 0 && !factsLoading ? (
                                <div className="text-xs text-[var(--color-text-tertiary)] italic">
                                    No facts stored yet for this entity. They will appear here as they are inferred from conversations.
                                </div>
                            ) : (
                                entityFacts.map((fact) => (
                                    <div
                                        key={fact.id}
                                        className="border border-[var(--color-border-secondary)] rounded-lg p-2 bg-[var(--color-bg-tertiary)]/40 flex flex-col gap-2 text-xs"
                                    >
                                        <div className="flex items-center justify-between gap-2">
                                            <div className="flex items-center gap-1 text-[var(--color-text-tertiary)]">
                                                <Brain className="w-3 h-3 text-purple-400" />
                                                <span className="font-mono text-[10px] truncate">{fact.id}</span>
                                            </div>
                                            <button
                                                onClick={async () => {
                                                    if (!confirm("Delete this fact?")) return;
                                                    try {
                                                        await invoke("delete_fact", { id: fact.id });
                                                        setEntityFacts(prev => prev.filter(f => f.id !== fact.id));
                                                    } catch (e) {
                                                        console.error("Failed to delete fact", e);
                                                        setFactsError(String(e));
                                                    }
                                                }}
                                                className="p-1 rounded hover:bg-red-900/40 text-[var(--color-text-tertiary)] hover:text-red-300"
                                                title="Delete fact"
                                            >
                                                <Trash2 className="w-3 h-3" />
                                            </button>
                                        </div>
                                        <div className="grid grid-cols-2 gap-2">
                                            <div>
                                                <label className="block text-[10px] uppercase tracking-wide text-[var(--color-text-tertiary)] mb-0.5">Subject</label>
                                                <input
                                                    className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] rounded px-2 py-1 text-[11px]"
                                                    value={fact.subject}
                                                    onChange={(e) => setEntityFacts(prev => prev.map(f => f.id === fact.id ? { ...f, subject: e.target.value } : f))}
                                                />
                                            </div>
                                            <div>
                                                <label className="block text-[10px] uppercase tracking-wide text-[var(--color-text-tertiary)] mb-0.5">Predicate</label>
                                                <input
                                                    className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] rounded px-2 py-1 text-[11px]"
                                                    value={fact.predicate}
                                                    onChange={(e) => setEntityFacts(prev => prev.map(f => f.id === fact.id ? { ...f, predicate: e.target.value } : f))}
                                                />
                                            </div>
                                        </div>
                                        <div className="grid grid-cols-2 gap-2 items-center">
                                            <div>
                                                <label className="block text-[10px] uppercase tracking-wide text-[var(--color-text-tertiary)] mb-0.5">Object</label>
                                                <input
                                                    className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] rounded px-2 py-1 text-[11px]"
                                                    value={fact.object}
                                                    onChange={(e) => setEntityFacts(prev => prev.map(f => f.id === fact.id ? { ...f, object: e.target.value } : f))}
                                                />
                                            </div>
                                            <div className="flex gap-2 items-center">
                                                <div className="flex-1">
                                                    <label className="block text-[10px] uppercase tracking-wide text-[var(--color-text-tertiary)] mb-0.5">Kind</label>
                                                    <select
                                                        className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] rounded px-2 py-1 text-[11px]"
                                                        value={fact.object_kind}
                                                        onChange={(e) => setEntityFacts(prev => prev.map(f => f.id === fact.id ? { ...f, object_kind: e.target.value as Fact["object_kind"] } : f))}
                                                    >
                                                        <option value="literal">literal</option>
                                                        <option value="entity">entity</option>
                                                    </select>
                                                </div>
                                                <div className="flex-1">
                                                    <label className="block text-[10px] uppercase tracking-wide text-[var(--color-text-tertiary)] mb-0.5">Confidence</label>
                                                    <input
                                                        type="number"
                                                        min={0}
                                                        max={1}
                                                        step={0.05}
                                                        className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] rounded px-2 py-1 text-[11px]"
                                                        value={fact.confidence}
                                                        onChange={(e) => {
                                                            const val = parseFloat(e.target.value);
                                                            setEntityFacts(prev => prev.map(f => f.id === fact.id ? { ...f, confidence: isNaN(val) ? f.confidence : val } : f));
                                                        }}
                                                    />
                                                </div>
                                            </div>
                                        </div>
                                        <div className="flex items-center justify-between mt-1">
                                            <div className="text-[10px] text-[var(--color-text-tertiary)]">
                                                <span className="mr-2">Created: {new Date(fact.created_at).toLocaleString()}</span>
                                            </div>
                                            <button
                                                onClick={async () => {
                                                    try {
                                                        await invoke("update_fact", {
                                                            id: fact.id,
                                                            subject: fact.subject,
                                                            predicate: fact.predicate,
                                                            object: fact.object,
                                                            object_kind: fact.object_kind,
                                                            confidence: fact.confidence,
                                                        });
                                                        setFactsError(null);
                                                    } catch (e) {
                                                        console.error("Failed to update fact", e);
                                                        setFactsError(String(e));
                                                    }
                                                }}
                                                className="inline-flex items-center gap-1 px-2 py-1 rounded bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] text-[10px] font-semibold hover:bg-[var(--color-hover-bg)]"
                                            >
                                                <Edit2 className="w-3 h-3" /> Save
                                            </button>
                                        </div>
                                    </div>
                                ))
                            )}
                        </div>
                    </div>

                    <div className="mt-6 border-t border-[var(--color-border-primary)] pt-4">
                        <button
                            onClick={handleRebuildIndex}
                            disabled={!(fullSettings.memory_enabled ?? true)}
                            className={`w-full py-2 px-4 rounded-lg border border-[var(--color-border-secondary)] text-sm font-bold transition-colors flex items-center justify-center gap-2
                                ${(fullSettings.memory_enabled ?? true)
                                    ? "bg-[var(--color-bg-tertiary)] hover:bg-[var(--color-hover-bg)] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]"
                                    : "opacity-50 cursor-not-allowed text-[var(--color-text-tertiary)]"
                                }`}
                        >
                            <Trash2 className="w-4 h-4" /> Rebuild Memory Index
                        </button>
                        <p className="text-xs text-[var(--color-text-tertiary)] mt-2 text-center">
                            Use this if search results seem stale or incorrect.
                        </p>
                    </div>
                </div>
            </div>

            <div className="bg-[var(--color-bg-secondary)] p-6 rounded-xl border border-[var(--color-border-primary)] shadow-xl mb-8">
                <h3 className="text-lg font-bold mb-4 flex items-center gap-2">
                    <Palette className="w-5 h-5 text-blue-500" /> Appearance
                </h3>
                <ThemeSelector />

                <div className="mt-8 border-t border-[var(--color-border-secondary)] pt-6">
                    <h3 className="text-lg font-semibold text-[var(--color-text-primary)] mb-4">Typography</h3>

                    <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                        {/* Interface Font */}
                        <div className="space-y-4">
                            <h4 className="font-medium text-[var(--color-text-secondary)]">Interface Font</h4>
                            <div className="flex gap-2">
                                <div className="flex-1">
                                    <CustomSelect
                                        value={fontSettings.interface_font}
                                        onChange={(val) => setFontSettings({ ...fontSettings, interface_font: val })}
                                        options={AVAILABLE_FONTS}
                                    />
                                </div>
                                <div className="w-[100px]">
                                    <CustomSelect
                                        value={String(fontSettings.interface_font_size)}
                                        onChange={(val) => setFontSettings({ ...fontSettings, interface_font_size: Number(val) })}
                                        options={FONT_SIZES}
                                    />
                                </div>
                                <div className="w-[120px]">
                                    <CustomSelect
                                        value={fontSettings.interface_font_weight}
                                        onChange={(val) => setFontSettings({ ...fontSettings, interface_font_weight: val })}
                                        options={FONT_WEIGHTS}
                                    />
                                </div>
                            </div>
                        </div>

                        {/* Chat Font */}
                        <div className="space-y-4">
                            <h4 className="font-medium text-[var(--color-text-secondary)]">Chat Font</h4>
                            <div className="flex gap-2">
                                <div className="flex-1">
                                    <CustomSelect
                                        value={fontSettings.chat_font}
                                        onChange={(val) => setFontSettings({ ...fontSettings, chat_font: val })}
                                        options={AVAILABLE_FONTS}
                                    />
                                </div>
                                <div className="w-[100px]">
                                    <CustomSelect
                                        value={String(fontSettings.chat_font_size)}
                                        onChange={(val) => setFontSettings({ ...fontSettings, chat_font_size: Number(val) })}
                                        options={FONT_SIZES}
                                    />
                                </div>
                                <div className="w-[120px]">
                                    <CustomSelect
                                        value={fontSettings.chat_font_weight}
                                        onChange={(val) => setFontSettings({ ...fontSettings, chat_font_weight: val })}
                                        options={FONT_WEIGHTS}
                                    />
                                </div>
                            </div>
                        </div>
                    </div>
                </div>

            </div>

            <h2 className="text-2xl font-bold mb-6 flex items-center gap-2 text-[var(--color-text-primary)]">
                <Server className="text-blue-500" /> MCP Servers
            </h2>

            <div className="grid gap-4 mb-8">
                {servers.map(s => (
                    <div
                        key={s.name}
                        className={`bg-[var(--color-bg-secondary)] p-4 rounded-lg border flex justify-between items-center shadow-lg ${s.status === 'connected'
                            ? 'border-[var(--color-border-primary)]'
                            : s.status === 'unknown'
                                ? 'border-[var(--color-border-secondary)]/50 bg-[var(--color-bg-primary)]/10'
                                : 'border-red-900/50 bg-red-950/10'
                            }`}
                    >
                        <div className="flex items-center gap-3">
                            <div
                                className={`w-2 h-2 rounded-full ${s.status === 'connected'
                                    ? 'bg-green-500 animate-pulse'
                                    : s.status === 'unknown'
                                        ? 'bg-[var(--color-text-tertiary)]'
                                        : 'bg-red-500'
                                    }`}
                            />
                            <div className="flex flex-col">
                                <span className="font-mono font-bold text-[var(--color-text-primary)]">{s.name}</span>
                                <span className="text-xs text-[var(--color-text-tertiary)]">
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
                                        ? 'text-[var(--color-text-secondary)] bg-[var(--color-text-tertiary)]/10'
                                        : 'text-red-400 bg-red-500/10'
                                    }`}
                            >
                                {s.status.toUpperCase()}
                            </div>
                            <button
                                onClick={() => openEditModal(s.name, s.config)}
                                className="p-2 hover:bg-[var(--color-bg-tertiary)] rounded-lg text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors"
                                title="Edit"
                            >
                                <Edit2 size={16} />
                            </button>
                            <button
                                onClick={() => handleDeleteServer(s.name)}
                                className="p-2 hover:bg-red-900/30 rounded-lg text-[var(--color-text-secondary)] hover:text-red-400 transition-colors"
                                title="Delete"
                            >
                                <Trash2 size={16} />
                            </button>
                        </div>
                    </div>
                ))}

                <button
                    onClick={openAddModal}
                    className="w-full py-4 border-2 border-dashed border-[var(--color-border-primary)] rounded-xl text-[var(--color-text-tertiary)] hover:border-[var(--color-border-secondary)] hover:text-[var(--color-text-secondary)] transition-colors flex items-center justify-center gap-2 font-semibold"
                >
                    <Plus className="w-5 h-5" /> Add MCP Server
                </button>
            </div>

            <div className="bg-[var(--color-bg-secondary)] p-6 rounded-xl border border-[var(--color-border-primary)] shadow-xl mb-8">
                <h3 className="text-lg font-bold mb-4 flex items-center gap-2">
                    <Plus className="w-5 h-5 text-blue-500" /> Model Providers
                </h3>

                <ProvidersSettings providers={providers} onChange={setProviders} />

                <div className="mt-6 flex justify-end">
                    <button
                        onClick={saveSettings}
                        className="btn-primary font-bold py-2 px-6 rounded-lg transition-all shadow-lg"
                    >
                        Save Configuration
                    </button>
                </div>
            </div>

            {/* Modal */}
            {
                isModalOpen && (
                    <div className="fixed inset-0 bg-black/80 flex items-center justify-center z-50 p-4">
                        <div className="bg-[var(--color-bg-secondary)] border border-[var(--color-border-secondary)] rounded-xl w-full max-w-lg shadow-2xl overflow-hidden">
                            <div className="p-6 border-b border-[var(--color-border-primary)] flex justify-between items-center">
                                <h3 className="text-xl font-bold">{editingServer ? 'Edit Server' : 'Add Server'}</h3>
                                <button onClick={() => setIsModalOpen(false)} className="text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)]">&times;</button>
                            </div>

                            <div className="p-6 space-y-4">
                                {!editingServer && (
                                    <div>
                                        <label className="block text-xs font-bold text-[var(--color-text-tertiary)] uppercase tracking-wider mb-1">Server Name</label>
                                        <input
                                            className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] rounded-lg p-3 text-sm focus:border-blue-500 focus:ring-1 focus:ring-blue-500 outline-none"
                                            value={name}
                                            onChange={e => setName(e.target.value)}
                                            placeholder="e.g. filesystem"
                                        />
                                    </div>
                                )}

                                <div>
                                    <label className="block text-xs font-bold text-[var(--color-text-tertiary)] uppercase tracking-wider mb-1">Transport Type</label>
                                    <div className="flex gap-2">
                                        <button
                                            onClick={() => setTransportType("stdio")}
                                            className={`flex-1 py-2 rounded-lg text-sm font-bold border ${transportType === "stdio" ? "btn-primary border-[var(--color-accent-primary)]" : "bg-[var(--color-bg-primary)] border-[var(--color-border-secondary)] text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-tertiary)]"}`}
                                        >
                                            Stdio (Local)
                                        </button>
                                        <button
                                            onClick={() => setTransportType("sse")}
                                            className={`flex-1 py-2 rounded-lg text-sm font-bold border ${transportType === "sse" ? "btn-primary border-[var(--color-accent-primary)]" : "bg-[var(--color-bg-primary)] border-[var(--color-border-secondary)] text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-tertiary)]"}`}
                                        >
                                            SSE (Remote)
                                        </button>
                                    </div>
                                </div>

                                {transportType === "stdio" ? (
                                    <>
                                        <div>
                                            <label className="block text-xs font-bold text-[var(--color-text-tertiary)] uppercase tracking-wider mb-1">Command</label>
                                            <input
                                                className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] rounded-lg p-3 text-sm font-mono focus:border-blue-500 focus:ring-1 focus:ring-blue-500 outline-none"
                                                value={command}
                                                onChange={e => setCommand(e.target.value)}
                                                placeholder="e.g. npx"
                                            />
                                        </div>
                                        <div>
                                            <label className="block text-xs font-bold text-[var(--color-text-tertiary)] uppercase tracking-wider mb-1">Arguments</label>
                                            <input
                                                className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] rounded-lg p-3 text-sm font-mono focus:border-blue-500 focus:ring-1 focus:ring-blue-500 outline-none"
                                                value={args}
                                                onChange={e => setArgs(e.target.value)}
                                                placeholder="-y, @modelcontextprotocol/server..."
                                            />
                                        </div>
                                        <div>
                                            <label className="block text-xs font-bold text-[var(--color-text-tertiary)] uppercase tracking-wider mb-1">Environment (KEY=VALUE)</label>
                                            <textarea
                                                className={`w-full bg-[var(--color-bg-primary)] border rounded-lg p-3 text-sm font-mono focus:ring-1 outline-none min-h-[120px] ${envErrors.length > 0 ? "border-red-600 focus:border-red-500 focus:ring-red-500" : "border-[var(--color-border-secondary)] focus:border-blue-500 focus:ring-blue-500"}`}
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
                                        <label className="block text-xs font-bold text-[var(--color-text-tertiary)] uppercase tracking-wider mb-1">SSE URL</label>
                                        <input
                                            className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] rounded-lg p-3 text-sm font-mono focus:border-blue-500 focus:ring-1 focus:ring-blue-500 outline-none"
                                            value={url}
                                            onChange={e => setUrl(e.target.value)}
                                            placeholder="http://localhost:3000/sse"
                                        />
                                    </div>
                                )}

                                <div>
                                    <label className="block text-xs font-bold text-[var(--color-text-tertiary)] uppercase tracking-wider mb-1">Allowlist (Comma separated)</label>
                                    <input
                                        className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] rounded-lg p-3 text-sm focus:border-blue-500 focus:ring-1 focus:ring-blue-500 outline-none"
                                        value={allowlist}
                                        onChange={e => setAllowlist(e.target.value)}
                                        placeholder="tool_a, tool_b (Leave empty to allow all)"
                                    />
                                </div>
                                <div>
                                    <label className="block text-xs font-bold text-[var(--color-text-tertiary)] uppercase tracking-wider mb-1">Denylist (Comma separated)</label>
                                    <input
                                        className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] rounded-lg p-3 text-sm focus:border-red-500 focus:ring-1 focus:ring-red-500 outline-none"
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

                            <div className="p-6 border-t border-[var(--color-border-primary)] flex justify-end gap-2">
                                <button
                                    onClick={() => setIsModalOpen(false)}
                                    className="px-4 py-2 rounded-lg hover:bg-[var(--color-bg-tertiary)] text-[var(--color-text-secondary)] font-bold text-sm"
                                >
                                    Cancel
                                </button>
                                <button
                                    onClick={handleSaveServer}
                                    className="px-6 py-2 rounded-lg btn-primary font-bold text-sm shadow-lg"
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
