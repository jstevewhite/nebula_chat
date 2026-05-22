import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Brain } from "lucide-react";

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

interface DocSummary {
    id: string;
    title: string;
    tags: string[];
    updated_at: string;
}

interface DocRecord {
    id: string;
    title: string;
    tags: string[];
    links: string[];
    content: string;
    created_at: string;
    updated_at: string;
}

export default function MemoryPanel({ memories, onClose: _onClose }: MemoryPanelProps) {
    const [activeTab, setActiveTab] = useState<"context" | "facts" | "docs">("context");
    const [userFacts, setUserFacts] = useState<FactRow[]>([]);
    const [entityKey, setEntityKey] = useState("");
    const [entityFacts, setEntityFacts] = useState<FactRow[]>([]);
    const [factsLoading, setFactsLoading] = useState(false);
    const [factsError, setFactsError] = useState<string | null>(null);
    const [docs, setDocs] = useState<DocSummary[]>([]);
    const [docsLoading, setDocsLoading] = useState(false);
    const [docsError, setDocsError] = useState<string | null>(null);
    const [selectedDoc, setSelectedDoc] = useState<DocRecord | null>(null);

    useEffect(() => {
        if (activeTab !== "docs") return;
        if (docs.length > 0 || docsLoading) return;
        (async () => {
            try {
                setDocsLoading(true);
                const rows = await invoke<DocSummary[]>("list_memory_docs");
                setDocs(rows);
                setDocsError(null);
            } catch (e) {
                console.error("Failed to load memory docs", e);
                setDocsError(String(e));
            } finally {
                setDocsLoading(false);
            }
        })();
    }, [activeTab, docs.length, docsLoading]);

    const openDoc = async (id: string) => {
        try {
            setDocsLoading(true);
            const doc = await invoke<DocRecord | null>("fetch_memory_doc", { id });
            setSelectedDoc(doc);
            setDocsError(null);
        } catch (e) {
            console.error("Failed to fetch doc", e);
            setDocsError(String(e));
        } finally {
            setDocsLoading(false);
        }
    };

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
        <div className="h-full flex flex-col bg-[var(--color-bg-secondary)]">
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
                        <button
                            className={`px-2 py-0.5 rounded-full border ${
                                activeTab === "docs"
                                    ? "bg-purple-600/40 border-purple-400 text-purple-50"
                                    : "bg-[var(--color-bg-tertiary)] border-[var(--color-border-secondary)] text-[var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)]"
                            }`}
                            onClick={() => setActiveTab("docs")}
                        >
                            Docs
                        </button>
                    </div>
                </div>
            </div>

            {activeTab === "context" && (
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
            )}

            {activeTab === "facts" && (
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

            {activeTab === "docs" && (
                <div className="flex-1 overflow-y-auto p-4 space-y-3 text-xs text-[var(--color-text-primary)]">
                    {docsError && (
                        <div className="text-red-300 bg-red-900/40 border border-red-700/60 rounded p-2">
                            {docsError}
                        </div>
                    )}

                    {selectedDoc ? (
                        <div className="space-y-2">
                            <div className="flex items-center justify-between">
                                <h4 className="text-[11px] font-semibold text-purple-300 truncate">
                                    {selectedDoc.title}
                                </h4>
                                <button
                                    onClick={() => setSelectedDoc(null)}
                                    className="text-[10px] underline text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]"
                                >
                                    Back to list
                                </button>
                            </div>
                            <div className="font-mono text-[10px] text-[var(--color-text-tertiary)]">
                                {selectedDoc.id} · updated {selectedDoc.updated_at}
                            </div>
                            {selectedDoc.tags.length > 0 && (
                                <div className="text-[10px] text-[var(--color-text-secondary)]">
                                    tags: {selectedDoc.tags.join(", ")}
                                </div>
                            )}
                            {selectedDoc.links.length > 0 && (
                                <div className="text-[10px] text-[var(--color-text-secondary)]">
                                    links: {selectedDoc.links.map((l) => (
                                        <button
                                            key={l}
                                            onClick={() => openDoc(l)}
                                            className="font-mono underline mr-2 hover:text-purple-300"
                                        >
                                            {l}
                                        </button>
                                    ))}
                                </div>
                            )}
                            <pre className="whitespace-pre-wrap bg-[var(--color-bg-tertiary)]/60 border border-[var(--color-border-secondary)] rounded p-2 text-[11px] text-[var(--color-text-primary)] font-mono">
                                {selectedDoc.content}
                            </pre>
                        </div>
                    ) : (
                        <>
                            <p className="text-[11px] text-[var(--color-text-secondary)]">
                                Memory documents are markdown files on disk under{" "}
                                <code className="font-mono">memory/docs/</code>. The LLM can
                                read and update them via the <code className="font-mono">memory_*</code> tools.
                            </p>
                            {docsLoading && docs.length === 0 ? (
                                <div className="text-[var(--color-text-secondary)] italic">Loading docs...</div>
                            ) : docs.length === 0 ? (
                                <div className="text-[var(--color-text-secondary)] italic">
                                    No memory documents yet.
                                </div>
                            ) : (
                                <ul className="space-y-1">
                                    {docs.map((d) => (
                                        <li
                                            key={d.id}
                                            className="border border-[var(--color-border-secondary)] rounded px-2 py-1 bg-[var(--color-bg-tertiary)]/60"
                                        >
                                            <button
                                                onClick={() => openDoc(d.id)}
                                                className="w-full text-left"
                                            >
                                                <div className="font-semibold text-[var(--color-text-primary)] truncate">
                                                    {d.title}
                                                </div>
                                                <div className="font-mono text-[10px] text-[var(--color-text-tertiary)] truncate">
                                                    {d.id} · {d.updated_at}
                                                </div>
                                                {d.tags.length > 0 && (
                                                    <div className="text-[10px] text-[var(--color-text-secondary)] truncate">
                                                        {d.tags.join(", ")}
                                                    </div>
                                                )}
                                            </button>
                                        </li>
                                    ))}
                                </ul>
                            )}
                        </>
                    )}
                </div>
            )}
        </div>
    );
}
