import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Plus, Trash2, Save, Book } from "lucide-react";

interface SystemPrompt {
    id: string;
    name: string;
    content: string;
}

export default function PromptsSettings() {
    const [prompts, setPrompts] = useState<SystemPrompt[]>([]);
    const [selectedId, setSelectedId] = useState<string | null>(null);
    const [name, setName] = useState("");
    const [content, setContent] = useState("");
    const [status, setStatus] = useState("");

    useEffect(() => {
        loadPrompts();
    }, []);

    const loadPrompts = async () => {
        try {
            const list = await invoke<SystemPrompt[]>("get_system_prompts");
            setPrompts(list);
        } catch (e) {
            console.error(e);
        }
    };

    const handleSelect = (p: SystemPrompt) => {
        setSelectedId(p.id);
        setName(p.name);
        setContent(p.content);
        setStatus("");
    };

    const handleNew = () => {
        setSelectedId(null);
        setName("");
        setContent("");
        setStatus("");
    };

    const handleSave = async () => {
        if (!name || !content) return;
        setStatus("Saving...");
        try {
            await invoke("save_system_prompt", {
                id: selectedId,
                name,
                content
            });
            await loadPrompts();
            setStatus("Saved!");
            if (!selectedId) {
                // If we improved the backend to return ID we could auto-select, 
                // but for now we'll just clear or find by name.
                // Let's just reset to new state for rapid entry or maybe stay?
                // Better UX: Find the new one.
                const list = await invoke<SystemPrompt[]>("get_system_prompts");
                const newItem = list.find(p => p.name === name && p.content === content);
                if (newItem) setSelectedId(newItem.id);
            }
        } catch (e: any) {
            setStatus("Error: " + e);
        }
    };

    const handleDelete = async (id: string, e: React.MouseEvent) => {
        e.stopPropagation();
        if (!confirm("Delete this prompt?")) return;
        try {
            await invoke("delete_system_prompt", { id });
            await loadPrompts();
            if (selectedId === id) handleNew();
        } catch (e) {
            console.error(e);
        }
    };

    return (
        <div className="flex h-[500px] border border-[var(--color-border-primary)] rounded-xl overflow-hidden bg-[var(--color-bg-secondary)]">
            {/* Sidebar List */}
            <div className="w-1/3 border-r border-[var(--color-border-primary)] flex flex-col">
                <div className="p-3 border-b border-[var(--color-border-primary)] flex justify-between items-center bg-[var(--color-bg-primary)]">
                    <span className="font-bold text-[var(--color-text-secondary)] text-xs uppercase tracking-wider">Prompts</span>
                    <button onClick={handleNew} className="p-1 hover:bg-[var(--color-bg-tertiary)] rounded text-[var(--color-accent-primary)]">
                        <Plus size={16} />
                    </button>
                </div>
                <div className="flex-1 overflow-y-auto p-2 space-y-1">
                    {prompts.map(p => (
                        <div
                            key={p.id}
                            onClick={() => handleSelect(p)}
                            className={`p-2 rounded cursor-pointer text-sm flex justify-between items-center group transition-colors ${selectedId === p.id ? "bg-[var(--color-bg-tertiary)] text-[var(--color-accent-primary)]" : "hover:bg-[var(--color-bg-tertiary)] text-[var(--color-text-secondary)]"}`}
                        >
                            <div className="flex items-center gap-2 truncate">
                                <Book size={14} className="opacity-50" />
                                <span className="truncate">{p.name}</span>
                            </div>
                            <button
                                onClick={(e) => handleDelete(p.id, e)}
                                className="opacity-0 group-hover:opacity-100 p-1 hover:text-red-400 transition-opacity"
                            >
                                <Trash2 size={14} />
                            </button>
                        </div>
                    ))}
                    {prompts.length === 0 && (
                        <div className="text-center text-[var(--color-text-tertiary)] text-xs py-10">
                            No prompts created.
                        </div>
                    )}
                </div>
            </div>

            {/* Editor */}
            <div className="flex-1 flex flex-col bg-[var(--color-bg-primary)]">
                <div className="p-4 border-b border-[var(--color-border-primary)] space-y-3">
                    <div>
                        <input
                            className="w-full bg-transparent text-lg font-bold placeholder-gray-600 outline-none"
                            placeholder="Prompt Name (e.g. Coding Assistant)"
                            value={name}
                            onChange={(e) => setName(e.target.value)}
                        />
                    </div>
                </div>
                <textarea
                    className="flex-1 w-full bg-[var(--color-bg-primary)] p-4 resize-none outline-none text-sm font-mono text-[var(--color-text-secondary)] leading-relaxed border-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500"
                    placeholder="Enter system prompt content here..."
                    value={content}
                    onChange={(e) => setContent(e.target.value)}
                    autoFocus={selectedId === null}
                    style={{fieldSizing: "content" as any}}
                />
                <div className="p-3 border-t border-[var(--color-border-primary)] flex justify-between items-center bg-[var(--color-bg-secondary)]">
                    <span className={`text-xs ${status.startsWith("Error") ? "text-red-400" : "text-green-400"}`}>
                        {status}
                    </span>
                    <button
                        onClick={handleSave}
                        disabled={!name || !content}
                        className="flex items-center gap-2 px-4 py-2 btn-primary disabled:opacity-50 disabled:cursor-not-allowed text-sm font-bold rounded-lg transition-colors"
                    >
                        <Save size={16} /> Save Prompt
                    </button>
                </div>
            </div>
        </div>
    );
}
