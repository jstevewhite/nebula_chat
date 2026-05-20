import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Brain, X } from "lucide-react";

interface MemoryPanelProps {
    memories: string[];
    onClose: () => void;
}

interface FactRow {
    id: string;
    subject: string;
    predicate: string;
    object: string;
    object_kind: "entity" | "literal";
    confidence: number;
    created_at: string;
    updated_at: string;
}

export default function MemoryPanel({ memories, onClose }: MemoryPanelProps) {
    const [activeTab, setActiveTab] = useState<"context" | "facts">("context");
    const [userFacts, setUserFacts] = useState<FactRow[]>([]);
    const [entityKey, setEntityKey] = useState("");
    const [entityFacts, setEntityFacts] = useState<FactRow[]>([]);
    const [factsLoading, setFactsLoading] = useState(false);
    const [factsError, setFactsError] = useState<string | null>(null);

    useEffect(() => {
        if (activeTab === "facts" && userFacts.length === 0 && !factsLoading) {
            (async () => {
                try {
                    setFactsLoading(true);
                    const facts = await invoke<FactRow[]>("list_user_facts");
                    setUserFacts(facts);
                    setFactsError(null);
                } catch (e) {
                    console.error("Failed to load user facts in MemoryPanel", e);
                    setFactsError(String(e));
                } finally {
                    setFactsLoading(false);
                }
            })();
        }
    }, [activeTab, userFacts.length, factsLoading]);

    const loadEntityFacts = async () => {
        const key = entityKey.trim();
        if (!key) return;
        try {
            setFactsLoading(true);
            const facts = await invoke<FactRow[]>("list_facts_for_entity", {
                entity: key,
                limit: 50,
            });
            setEntityFacts(facts);
            setFactsError(null);
        } catch (e) {
            console.error("Failed to load entity facts", e);
            setFactsError(String(e));
        } finally {
            setFactsLoading(false);
        }
    };

    return (
        <div className="w-80 h-full border-l border-[var(--color-border-primary)] bg-[var(--color-bg-secondary)] flex flex-col shadow-xl shrink-0 animate-in slide-in-from-right duration-200">
            <div className="p-4 border-b border-[var(--color-border-primary)] flex justify-between items-center bg-[var(--color-bg-secondary)]/50 backdrop-blur">
                <div className="flex flex-col gap-1">
                    <h3 className="text-sm font-semibold text-[var(--color-text-primary)] flex items-center gap-2">
                        <Brain size={16} className="text-purple-400" />
                        Memory
                    </h3>
                    <div className="flex gap-2 text-xs">
                        <button
                            className={`px-2 py-0.5 rounded-full border ${
                                activeTab === "context"
                                    ? "bg-purple-600/40 border-purple-400 text-purple-50"
                                    : "bg-[var(--color-bg-tertiary)] border-[var(--color-border-secondary)] text-[var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)]"
                            }`}
                            onClick={() => setActiveTab("context")}
                        >
                            Context
                        </button>
                        <button
                            className={`px-2 py-0.5 rounded-full border ${
                                activeTab === "facts"
                                    ? "bg-purple-600/40 border-purple-400 text-purple-50"
                                    : "bg-[var(--color-bg-tertiary)] border-[var(--color-border-secondary)] text-[var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)]"
                            }`}
                            onClick={() => setActiveTab("facts")}
                        >
                            Facts
                        </button>
                    </div>
                </div>
                <button
                    onClick={onClose}
                    className="text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors p-1 hover:bg-[var(--color-bg-tertiary)] rounded"
                >
                    <X size={16} />
                </button>
            </div>

            {activeTab === "context" ? (
                <div className="flex-1 overflow-y-auto p-4 space-y-3">
                    {memories.length === 0 ? (
                        <div className="text-center text-[var(--color-text-secondary)] mt-10 text-sm italic">
                            No active memories found for this interaction.
                        </div>
                    ) : (
                        memories.map((mem, i) => (
                            <div
                                key={i}
                                className="bg-[var(--color-bg-tertiary)]/50 border border-[var(--color-border-secondary)] p-3 rounded-lg text-sm text-[var(--color-text-primary)] shadow-sm hover:border-purple-500/30 transition-colors"
                            >
                                <div className="text-xs text-purple-400 mb-1 font-mono opacity-75">
                                    MEMORY FRAGMENT {i + 1}
                                </div>
                                "{mem}"
                            </div>
                        ))
                    )}
                </div>
            ) : (
                <div className="flex-1 overflow-y-auto p-4 space-y-3 text-xs text-[var(--color-text-primary)]">
                    {factsError && (
                        <div className="text-red-300 bg-red-900/40 border border-red-700/60 rounded p-2">
                            {factsError}
                        </div>
                    )}

                    <div>
                        <h4 className="text-[11px] font-semibold text-purple-300 mb-1">User Profile Facts</h4>
                        {factsLoading && userFacts.length === 0 ? (
                            <div className="text-[var(--color-text-secondary)] italic">Loading facts...</div>
                        ) : userFacts.length === 0 ? (
                            <div className="text-[var(--color-text-secondary)] italic">
                                No user profile facts stored yet.
                            </div>
                        ) : (
                            <ul className="space-y-1">
                                {userFacts.map((f) => (
                                    <li
                                        key={f.id}
                                        className="border border-[var(--color-border-secondary)] rounded px-2 py-1 bg-[var(--color-bg-tertiary)]/60"
                                    >
                                        <div className="font-mono text-[10px] text-[var(--color-text-tertiary)] truncate mb-0.5">
                                            {f.id}
                                        </div>
                                        <div>
                                            <span className="font-semibold text-[var(--color-text-primary)]">{f.subject}</span>{" "}
                                            <span className="text-[var(--color-text-secondary)]">{f.predicate}</span>{" "}
                                            <span className="text-[var(--color-text-primary)]">{f.object}</span>{" "}
                                            <span className="text-[var(--color-text-tertiary)]">
                                                ({f.object_kind}, conf={f.confidence.toFixed(2)})
                                            </span>
                                        </div>
                                    </li>
                                ))}
                            </ul>
                        )}
                    </div>

                    <div className="pt-2 border-t border-[var(--color-border-primary)]">
                        <h4 className="text-[11px] font-semibold text-purple-300 mb-1">Facts About Entity</h4>
                        <p className="text-[11px] text-[var(--color-text-secondary)] mb-2">
                            Enter an entity key (e.g. <code className="font-mono">nebula_chat</code>,
                            <code className="font-mono">tauri</code>) to inspect related facts.
                        </p>
                        <div className="flex gap-2 mb-2">
                            <input
                                className="flex-1 bg-[var(--color-bg-primary)] border border-[var(--color-border-secondary)] rounded px-2 py-1 text-[11px] text-[var(--color-text-primary)]"
                                placeholder="entity key (subject/object)"
                                value={entityKey}
                                onChange={(e) => setEntityKey(e.target.value)}
                            />
                            <button
                                onClick={loadEntityFacts}
                                disabled={factsLoading || !entityKey.trim()}
                                className="px-3 py-1 rounded bg-purple-600 text-white text-[11px] font-semibold disabled:opacity-50 disabled:cursor-not-allowed"
                            >
                                Load
                            </button>
                        </div>
                        {factsLoading && entityFacts.length === 0 && entityKey.trim() && (
                            <div className="text-[var(--color-text-secondary)] italic mb-1">Loading entity facts...</div>
                        )}
                        {entityKey.trim() && entityFacts.length === 0 && !factsLoading ? (
                            <div className="text-[var(--color-text-secondary)] italic">
                                No facts found for <span className="font-mono">{entityKey.trim()}</span>.
                            </div>
                        ) : null}
                        {entityFacts.length > 0 && (
                            <ul className="space-y-1 mt-1">
                                {entityFacts.map((f) => (
                                    <li
                                        key={f.id}
                                        className="border border-[var(--color-border-secondary)] rounded px-2 py-1 bg-[var(--color-bg-tertiary)]/60"
                                    >
                                        <div className="font-mono text-[10px] text-[var(--color-text-tertiary)] truncate mb-0.5">
                                            {f.id}
                                        </div>
                                        <div>
                                            <span className="font-semibold text-[var(--color-text-primary)]">{f.subject}</span>{" "}
                                            <span className="text-[var(--color-text-secondary)]">{f.predicate}</span>{" "}
                                            <span className="text-[var(--color-text-primary)]">{f.object}</span>{" "}
                                            <span className="text-[var(--color-text-tertiary)]">
                                                ({f.object_kind}, conf={f.confidence.toFixed(2)})
                                            </span>
                                        </div>
                                    </li>
                                ))}
                            </ul>
                        )}
                    </div>
                </div>
            )}
        </div>
    );
}
