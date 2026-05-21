import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Plus, Trash2, Save, Sparkles, Lock } from "lucide-react";

interface SkillSummary {
    slug: string;
    name: string;
    description: string;
    built_in: boolean;
    path: string;
}

interface Skill extends SkillSummary {
    body: string;
}

export default function SkillsSettings() {
    const [skills, setSkills] = useState<SkillSummary[]>([]);
    const [selectedSlug, setSelectedSlug] = useState<string | null>(null);
    const [editingNewSlug, setEditingNewSlug] = useState<string | null>(null);
    const [slug, setSlug] = useState("");
    const [name, setName] = useState("");
    const [description, setDescription] = useState("");
    const [body, setBody] = useState("");
    const [builtIn, setBuiltIn] = useState(false);
    const [status, setStatus] = useState("");

    const loadSkills = async () => {
        try {
            const list = await invoke<SkillSummary[]>("list_skills");
            setSkills(list);
        } catch (e) {
            console.error(e);
        }
    };

    useEffect(() => {
        loadSkills();
        // The backend FS watcher emits `skills-updated` whenever a file under
        // skills/ changes (debounced). Refresh the list so vim/git-pull edits
        // surface here without a manual reload.
        const unlisten = listen("skills-updated", () => {
            loadSkills();
        });
        return () => {
            unlisten.then((fn) => fn());
        };
    }, []);

    const handleSelect = async (sum: SkillSummary) => {
        try {
            const full = await invoke<Skill | null>("get_skill", { slug: sum.slug });
            if (!full) return;
            setSelectedSlug(full.slug);
            setEditingNewSlug(null);
            setSlug(full.slug);
            setName(full.name);
            setDescription(full.description);
            setBody(full.body);
            setBuiltIn(full.built_in);
            setStatus("");
        } catch (e) {
            console.error(e);
            setStatus(`Error: ${e}`);
        }
    };

    const handleNew = () => {
        setSelectedSlug(null);
        setEditingNewSlug("");
        setSlug("");
        setName("");
        setDescription("");
        setBody("");
        setBuiltIn(false);
        setStatus("");
    };

    const handleSave = async () => {
        if (!slug || !name || !description || !body) {
            setStatus("Error: slug, name, description, and body are all required");
            return;
        }
        setStatus("Saving...");
        try {
            if (selectedSlug) {
                await invoke("update_skill", { slug: selectedSlug, name, description, body });
            } else {
                await invoke("create_skill", { slug, name, description, body });
                setSelectedSlug(slug);
                setEditingNewSlug(null);
            }
            await loadSkills();
            setStatus("Saved");
            setTimeout(() => setStatus((s) => (s === "Saved" ? "" : s)), 1500);
        } catch (e) {
            setStatus(`Error: ${e}`);
        }
    };

    const handleDelete = async (s: SkillSummary, e: React.MouseEvent) => {
        e.stopPropagation();
        const verb = s.built_in
            ? "Delete this built-in skill from disk? It will be regenerated on the next app start."
            : "Delete this skill?";
        if (!confirm(verb)) return;
        try {
            await invoke("delete_skill", { slug: s.slug });
            await loadSkills();
            if (selectedSlug === s.slug) handleNew();
        } catch (e) {
            console.error(e);
        }
    };

    const slugDisabled = selectedSlug !== null; // can't rename in place

    return (
        <div className="flex h-[500px] border border-[var(--color-border-primary)] rounded-xl overflow-hidden bg-[var(--color-bg-secondary)]">
            {/* Sidebar List */}
            <div className="w-1/3 border-r border-[var(--color-border-primary)] flex flex-col">
                <div className="p-3 border-b border-[var(--color-border-primary)] flex justify-between items-center bg-[var(--color-bg-primary)]">
                    <span className="font-bold text-[var(--color-text-secondary)] text-xs uppercase tracking-wider">Skills</span>
                    <button
                        onClick={handleNew}
                        className="p-1 hover:bg-[var(--color-bg-tertiary)] rounded text-[var(--color-accent-primary)]"
                        title="New skill"
                    >
                        <Plus size={16} />
                    </button>
                </div>
                <div className="flex-1 overflow-y-auto p-2 space-y-1">
                    {skills.map((s) => (
                        <div
                            key={s.slug}
                            onClick={() => handleSelect(s)}
                            className={`p-2 rounded cursor-pointer text-sm flex justify-between items-center group transition-colors ${
                                selectedSlug === s.slug
                                    ? "bg-[var(--color-bg-tertiary)] text-[var(--color-accent-primary)]"
                                    : "hover:bg-[var(--color-bg-tertiary)] text-[var(--color-text-secondary)]"
                            }`}
                        >
                            <div className="flex items-center gap-2 truncate min-w-0">
                                <Sparkles size={14} className="opacity-50 shrink-0" />
                                <span className="truncate">{s.name}</span>
                                {s.built_in && (
                                    <span className="shrink-0 text-[9px] uppercase tracking-wider bg-purple-500/20 text-purple-300 px-1 rounded">
                                        built-in
                                    </span>
                                )}
                            </div>
                            <button
                                onClick={(ev) => handleDelete(s, ev)}
                                className="p-1 hover:text-red-400 transition-opacity"
                                title="Delete"
                            >
                                <Trash2 size={14} />
                            </button>
                        </div>
                    ))}
                    {skills.length === 0 && (
                        <div className="text-center text-[var(--color-text-tertiary)] text-xs py-10">
                            No skills installed.
                        </div>
                    )}
                </div>
            </div>

            {/* Editor */}
            <div className="flex-1 flex flex-col bg-[var(--color-bg-primary)]">
                {selectedSlug === null && editingNewSlug === null ? (
                    <div className="flex-1 flex items-center justify-center text-sm text-[var(--color-text-tertiary)]">
                        Select a skill on the left, or click + to create one.
                    </div>
                ) : (
                    <>
                        <div className="p-4 border-b border-[var(--color-border-primary)] space-y-3">
                            <div className="flex items-center gap-2">
                                <input
                                    className="flex-1 bg-transparent text-lg font-bold placeholder-gray-600 outline-none"
                                    placeholder="Skill name (e.g. Code Review)"
                                    value={name}
                                    onChange={(e) => setName(e.target.value)}
                                />
                                {builtIn && (
                                    <span className="flex items-center gap-1 text-[10px] uppercase tracking-wider bg-purple-500/20 text-purple-300 px-2 py-0.5 rounded">
                                        <Lock size={10} /> built-in
                                    </span>
                                )}
                            </div>
                            <input
                                className="w-full bg-transparent text-xs text-[var(--color-text-tertiary)] outline-none border border-[var(--color-border-secondary)] rounded px-2 py-1 font-mono disabled:opacity-50"
                                placeholder="slug (e.g. code-review) — kebab-case, used in use_skill calls"
                                value={slug}
                                disabled={slugDisabled}
                                onChange={(e) => setSlug(e.target.value)}
                            />
                            <input
                                className="w-full bg-transparent text-sm outline-none border border-[var(--color-border-secondary)] rounded px-2 py-1"
                                placeholder="When should the LLM use this skill?"
                                value={description}
                                onChange={(e) => setDescription(e.target.value)}
                            />
                            {builtIn && (
                                <p className="text-[10px] text-[var(--color-text-tertiary)]">
                                    Edits to built-in skills stick. To restore the original, delete the skill — it regenerates on app start.
                                </p>
                            )}
                        </div>
                        <textarea
                            className="flex-1 w-full bg-[var(--color-bg-primary)] p-4 resize-none outline-none text-sm font-mono text-[var(--color-text-secondary)] leading-relaxed border-none focus:border-blue-500"
                            placeholder="System-prompt-shaped body. Treated as authoritative guidance when the model calls use_skill."
                            value={body}
                            onChange={(e) => setBody(e.target.value)}
                        />
                        <div className="p-3 border-t border-[var(--color-border-primary)] flex justify-between items-center bg-[var(--color-bg-secondary)]">
                            <span className={`text-xs ${status.startsWith("Error") ? "text-red-400" : "text-green-400"}`}>
                                {status}
                            </span>
                            <button
                                onClick={handleSave}
                                disabled={!slug || !name || !description || !body}
                                className="flex items-center gap-2 px-4 py-2 btn-primary disabled:opacity-50 disabled:cursor-not-allowed text-sm font-bold rounded-lg transition-colors"
                            >
                                <Save size={16} /> Save
                            </button>
                        </div>
                    </>
                )}
            </div>
        </div>
    );
}
