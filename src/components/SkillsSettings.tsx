import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Plus, Trash2, Save, Sparkles, Lock, Copy } from "lucide-react";

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

interface ClaudeSkillEntry {
    slug: string;
    name: string;
    description: string;
    heuristic_default: boolean;
    effective_enabled: boolean;
    shadowed_by_native: boolean;
}

// Minimal shape of the parts of Settings this component reads/writes. The
// backend's save_settings takes the full Settings object, so we load it whole,
// mutate these fields, and save it back to avoid clobbering other settings.
type Settings = Record<string, unknown> & {
    import_claude_skills?: boolean;
    claude_skill_overrides?: Record<string, boolean>;
};

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
    const [importClaude, setImportClaude] = useState(false);
    const [claudeSkills, setClaudeSkills] = useState<ClaudeSkillEntry[]>([]);

    const loadSkills = async () => {
        try {
            const list = await invoke<SkillSummary[]>("list_skills");
            setSkills(list);
        } catch (e) {
            console.error(e);
        }
    };

    const loadClaudeImport = async () => {
        try {
            const settings = await invoke<Settings>("get_settings");
            setImportClaude(!!settings.import_claude_skills);
            if (settings.import_claude_skills) {
                setClaudeSkills(await invoke<ClaudeSkillEntry[]>("list_claude_skills"));
            } else {
                setClaudeSkills([]);
            }
        } catch (e) {
            console.error(e);
        }
    };

    useEffect(() => {
        loadSkills();
        loadClaudeImport();
        // The backend FS watcher emits `skills-updated` whenever a file under
        // skills/ (or, when enabled, ~/.claude/skills) changes, and after the
        // import toggle/overrides are saved. Refresh both views.
        const unlisten = listen("skills-updated", () => {
            loadSkills();
            loadClaudeImport();
        });
        return () => {
            unlisten.then((fn) => fn());
        };
    }, []);

    const persistSettings = async (mutate: (s: Settings) => void) => {
        const settings = await invoke<Settings>("get_settings");
        mutate(settings);
        await invoke("save_settings", { settings });
        // save_settings emits skills-updated, which triggers loadClaudeImport.
    };

    const handleToggleImport = async () => {
        const next = !importClaude;
        setImportClaude(next); // optimistic
        try {
            await persistSettings((s) => {
                s.import_claude_skills = next;
            });
        } catch (e) {
            console.error(e);
            setImportClaude(!next); // revert on failure
        }
    };

    const handleToggleClaudeSkill = async (entry: ClaudeSkillEntry) => {
        if (entry.shadowed_by_native) return;
        const next = !entry.effective_enabled;
        try {
            await persistSettings((s) => {
                const overrides = { ...(s.claude_skill_overrides ?? {}) };
                overrides[entry.slug] = next;
                s.claude_skill_overrides = overrides;
            });
        } catch (e) {
            console.error(e);
        }
    };

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
        if (builtIn) return;
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

    const handleClone = async () => {
        // Built-in is selected — create a user-level skill seeded with its
        // contents. Slug gets `-copy` (or `-copy-2`, `-copy-3`, ...) appended
        // to keep it valid kebab-case and unique against existing skills.
        const existingSlugs = new Set(skills.map((s) => s.slug));
        let cloneSlug = `${slug}-copy`;
        let suffix = 2;
        while (existingSlugs.has(cloneSlug)) {
            cloneSlug = `${slug}-copy-${suffix}`;
            suffix += 1;
        }
        const cloneName = `${name} (Copy)`;
        setStatus("Cloning...");
        try {
            await invoke("create_skill", {
                slug: cloneSlug,
                name: cloneName,
                description,
                body,
            });
            await loadSkills();
            // Switch to the new user-owned copy so the user can edit immediately.
            setSelectedSlug(cloneSlug);
            setEditingNewSlug(null);
            setSlug(cloneSlug);
            setName(cloneName);
            setBuiltIn(false);
            setStatus("Cloned — editing your copy now.");
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
      <div className="space-y-4">
        {/* Claude Code skills import */}
        <div className="border border-[var(--color-border-primary)] rounded-xl bg-[var(--color-bg-secondary)] p-4 space-y-3">
            <label className="flex items-center justify-between gap-3 cursor-pointer">
                <span>
                    <span className="font-bold text-sm text-[var(--color-text-primary)]">
                        Import skills from Claude Code
                    </span>
                    <span className="block text-xs text-[var(--color-text-tertiary)] font-mono">
                        ~/.claude/skills
                    </span>
                </span>
                <input
                    type="checkbox"
                    checked={importClaude}
                    onChange={handleToggleImport}
                    className="h-4 w-4 shrink-0"
                />
            </label>

            {importClaude && (
                <div className="space-y-1 max-h-48 overflow-y-auto pt-1 border-t border-[var(--color-border-secondary)]">
                    {claudeSkills.length === 0 && (
                        <div className="text-center text-[var(--color-text-tertiary)] text-xs py-6">
                            No Claude skills found in ~/.claude/skills.
                        </div>
                    )}
                    {claudeSkills.map((c) => (
                        <label
                            key={c.slug}
                            className={`flex items-start gap-2 p-2 rounded text-sm ${
                                c.shadowed_by_native ? "opacity-50" : "hover:bg-[var(--color-bg-tertiary)] cursor-pointer"
                            }`}
                        >
                            <input
                                type="checkbox"
                                checked={c.effective_enabled}
                                disabled={c.shadowed_by_native}
                                onChange={() => handleToggleClaudeSkill(c)}
                                className="h-4 w-4 mt-0.5 shrink-0"
                            />
                            <span className="min-w-0">
                                <span className="flex items-center gap-2">
                                    <span className="truncate text-[var(--color-text-secondary)]">{c.name}</span>
                                    <span className="shrink-0 text-[9px] uppercase tracking-wider bg-blue-500/20 text-blue-300 px-1 rounded">
                                        from Claude Code
                                    </span>
                                </span>
                                <span className="block text-xs text-[var(--color-text-tertiary)] truncate">
                                    {c.shadowed_by_native
                                        ? "Shadowed by a native skill"
                                        : !c.heuristic_default && !c.effective_enabled
                                        ? "Looks like it needs scripts — off by default"
                                        : c.description}
                                </span>
                            </span>
                        </label>
                    ))}
                </div>
            )}
        </div>

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
                            {!s.built_in && (
                                <button
                                    onClick={(ev) => handleDelete(s, ev)}
                                    className="p-1 hover:text-red-400 transition-opacity"
                                    title="Delete"
                                >
                                    <Trash2 size={14} />
                                </button>
                            )}
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
                                    className="flex-1 bg-transparent text-lg font-bold placeholder-gray-600 outline-none disabled:opacity-70 disabled:cursor-not-allowed"
                                    placeholder="Skill name (e.g. Code Review)"
                                    value={name}
                                    onChange={(e) => setName(e.target.value)}
                                    disabled={builtIn}
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
                                className="w-full bg-transparent text-sm outline-none border border-[var(--color-border-secondary)] rounded px-2 py-1 disabled:opacity-70 disabled:cursor-not-allowed"
                                placeholder="When should the LLM use this skill?"
                                value={description}
                                onChange={(e) => setDescription(e.target.value)}
                                disabled={builtIn}
                            />
                            {builtIn && (
                                <p className="text-[10px] text-[var(--color-text-tertiary)]">
                                    Built-in skills are re-applied from the binary on every launch — edits would be overwritten. Click <span className="font-bold">Clone to edit</span> below to create your own editable copy.
                                </p>
                            )}
                        </div>
                        <textarea
                            className="flex-1 w-full bg-[var(--color-bg-primary)] p-4 resize-none outline-none text-sm font-mono text-[var(--color-text-secondary)] leading-relaxed border-none focus:border-blue-500 read-only:opacity-80 read-only:cursor-default"
                            placeholder="System-prompt-shaped body. Treated as authoritative guidance when the model calls use_skill."
                            value={body}
                            onChange={(e) => setBody(e.target.value)}
                            readOnly={builtIn}
                        />
                        <div className="p-3 border-t border-[var(--color-border-primary)] flex justify-between items-center bg-[var(--color-bg-secondary)]">
                            <span className={`text-xs ${status.startsWith("Error") ? "text-red-400" : "text-green-400"}`}>
                                {status}
                            </span>
                            {builtIn ? (
                                <button
                                    onClick={handleClone}
                                    className="flex items-center gap-2 px-4 py-2 btn-primary text-sm font-bold rounded-lg transition-colors"
                                >
                                    <Copy size={16} /> Clone to edit
                                </button>
                            ) : (
                                <button
                                    onClick={handleSave}
                                    disabled={!slug || !name || !description || !body}
                                    className="flex items-center gap-2 px-4 py-2 btn-primary disabled:opacity-50 disabled:cursor-not-allowed text-sm font-bold rounded-lg transition-colors"
                                >
                                    <Save size={16} /> Save
                                </button>
                            )}
                        </div>
                    </>
                )}
            </div>
        </div>
          </div>
    );
}
