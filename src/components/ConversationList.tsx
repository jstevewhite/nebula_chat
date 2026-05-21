
import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Plus, MessageSquare, Trash2, Search, Upload, Minimize2 } from "lucide-react";

interface Conversation {
    id: string;
    title: string;
    icon?: string;
    created_at: string;
}

interface SearchResult {
    message_id: string;
    conversation_id: string;
    role: string;
    content: string;
    created_at: string;
    score: number;
}

interface ConversationListProps {
    activeId: string | null;
    onSelect: (id: string) => void;
    onCreate: () => void;
}

export default function ConversationList({ activeId, onSelect, onCreate }: ConversationListProps) {
    const [conversations, setConversations] = useState<Conversation[]>([]);
    const [filteredConversations, setFilteredConversations] = useState<Conversation[]>([]);
    const [searchResults, setSearchResults] = useState<SearchResult[]>([]);
    const [loading, setLoading] = useState(false);
    const [searchQuery, setSearchQuery] = useState("");
    const importRef = useRef<HTMLInputElement>(null);

    // Rename/Delete State
    const [editingId, setEditingId] = useState<string | null>(null);
    const [editTitle, setEditTitle] = useState("");
    const [hoveredId, setHoveredId] = useState<string | null>(null);
    const [deleteTarget, setDeleteTarget] = useState<Conversation | null>(null);
    const [error, setError] = useState<string | null>(null);
    
    // Icon editing state
    const [editingIconId, setEditingIconId] = useState<string | null>(null);
    const [editIcon, setEditIcon] = useState("");

    // Width & Compact Mode State
    const [width, setWidth] = useState(() => {
        const saved = localStorage.getItem("conversationListWidth");
        return saved ? parseInt(saved) : 256;
    });
    const [isResizing, setIsResizing] = useState(false);
    const [compactMode, setCompactMode] = useState(() => {
        const saved = localStorage.getItem("conversationListCompact");
        return saved === "true";
    });

    const loadConversations = async () => {
        setLoading(true);
        try {
            const list = await invoke<Conversation[]>("list_conversations");
            setConversations(list);
            setFilteredConversations(
                list.filter(c => c.title.toLowerCase().includes(searchQuery.toLowerCase()))
            );
        } catch (e) {
            console.error(e);
        } finally {
            setLoading(false);
        }
    };



    useEffect(() => {
        loadConversations();

        const unlistenPromise = listen("conversations-updated", () => {
            console.log("Conversations updated event received");
            loadConversations();
        });

        return () => {
            unlistenPromise.then((unlisten: () => void) => unlisten());
        };
    }, [activeId]);

    useEffect(() => {
        const performSearch = async () => {
            // Local Title Search
            setFilteredConversations(
                conversations.filter(c => c.title.toLowerCase().includes(searchQuery.toLowerCase()))
            );

            // Global Content Search (Debounced ideally, but here direct for now)
            if (searchQuery.length > 2) {
                try {
                    const results = await invoke<SearchResult[]>("search_messages", { query: searchQuery });
                    setSearchResults(results);
                } catch (e) {
                    console.error("Search failed", e);
                }
            } else {
                setSearchResults([]);
            }
        };
        performSearch();
    }, [searchQuery, conversations]);

    const handleImport = async (e: React.ChangeEvent<HTMLInputElement>) => {
        const file = e.target.files?.[0];
        if (!file) return;
        const reader = new FileReader();
        reader.onload = async (ev) => {
            const content = ev.target?.result as string;
            try {
                const newId = await invoke<string>("import_conversation", { jsonContent: content });
                await loadConversations();
                onSelect(newId);
                // Clear input
                if (importRef.current) importRef.current.value = "";
            } catch (err) {
                console.error(err);
                setError("Import failed: " + String(err));
            }
        };
        reader.readAsText(file);
    };

    const handleDelete = (e: React.MouseEvent, conv: Conversation) => {
        e.stopPropagation();
        setError(null);
        setDeleteTarget(conv);
    };

    const confirmDelete = async () => {
        if (!deleteTarget) return;
        const id = deleteTarget.id;
        try {
            await invoke("delete_conversation", { conversationId: id });

            // If we deleted the active conversation, decide what to select next
            if (activeId === id) {
                const remaining = conversations.filter(c => c.id !== id);
                if (remaining.length > 0) {
                    onSelect(remaining[0].id);
                } else {
                    onCreate();
                }
            }

            setDeleteTarget(null);
            loadConversations();
        } catch (e) {
            console.error(e);
            setError("Failed to delete chat: " + String(e));
        }
    };

    const startRename = (e: React.MouseEvent, conv: Conversation) => {
        e.stopPropagation();
        setEditingId(conv.id);
        setEditTitle(conv.title);
    };

    const saveRename = async () => {
        if (!editingId) return;
        try {
            await invoke("rename_conversation", { conversationId: editingId, newTitle: editTitle });
            loadConversations();
        } catch (e) {
            console.error(e);
        } finally {
            setEditingId(null);
        }
    };

    const startEditIcon = (e: React.MouseEvent, conv: Conversation) => {
        e.stopPropagation();
        setEditingIconId(conv.id);
        setEditIcon(conv.icon || "");
    };

    const saveIcon = async () => {
        if (!editingIconId) return;
        try {
            const iconToSave = editIcon.trim() === "" ? null : editIcon.trim();
            await invoke("update_conversation_icon", { conversationId: editingIconId, icon: iconToSave });
            loadConversations();
        } catch (e) {
            console.error(e);
        } finally {
            setEditingIconId(null);
        }
    };

    const formatDate = (dateStr: string) => {
        const date = new Date(dateStr);
        return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    }

    // Handle resize drag
    useEffect(() => {
        const handleMouseMove = (e: MouseEvent) => {
            if (!isResizing) return;
            const newWidth = Math.max(200, Math.min(500, e.clientX - 64)); // 64px for activity bar
            setWidth(newWidth);
            localStorage.setItem("conversationListWidth", newWidth.toString());
        };

        const handleMouseUp = () => {
            setIsResizing(false);
        };

        if (isResizing) {
            document.addEventListener("mousemove", handleMouseMove);
            document.addEventListener("mouseup", handleMouseUp);
        }

        return () => {
            document.removeEventListener("mousemove", handleMouseMove);
            document.removeEventListener("mouseup", handleMouseUp);
        };
    }, [isResizing]);

    const toggleCompactMode = () => {
        const newCompact = !compactMode;
        setCompactMode(newCompact);
        localStorage.setItem("conversationListCompact", newCompact.toString());
    };

    return (
        <div 
            className="bg-[var(--color-bg-secondary)] border-r border-[var(--color-border-primary)] flex flex-col h-full select-none relative"
            style={{ width: `${width}px` }}
        >
            <input type="file" ref={importRef} onChange={handleImport} className="hidden" accept=".json" />
            <div className="p-4 border-b border-[var(--color-border-primary)] space-y-3 relative">
                {error && (
                    <div className="text-xs bg-red-900/20 border border-red-700/40 text-red-200 rounded-lg px-3 py-2">
                        {error}
                    </div>
                )}
                <div className="flex gap-2">
                    <button
                        onClick={onCreate}
                        className="flex-1 btn-primary rounded-lg p-2.5 flex items-center justify-center gap-2 transition-all font-semibold text-sm shadow-md"
                    >
                        <Plus size={18} /> New Chat
                    </button>
                    <button
                        onClick={() => importRef.current?.click()}
                        className="bg-[var(--color-bg-tertiary)] hover:bg-gray-700 text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] rounded-lg p-2.5 flex items-center justify-center transition-all shadow-md"
                        title="Import JSON"
                    >
                        <Upload size={18} />
                    </button>
                </div>

                <div className="flex items-center gap-2">
                    <div className="relative flex-1">
                        <Search className="absolute left-2.5 top-2.5 text-[var(--color-text-tertiary)] w-4 h-4" />
                        <input
                            type="text"
                            placeholder="Search chats..."
                            value={searchQuery}
                            onChange={(e) => setSearchQuery(e.target.value)}
                            className="w-full bg-[var(--color-bg-primary)] border border-[var(--color-border-primary)] rounded-lg py-2 pl-9 pr-3 text-sm text-gray-200 focus:outline-none focus:border-blue-500/50 transition-colors placeholder-gray-600"
                        />
                    </div>
                    <button
                        onClick={toggleCompactMode}
                        className={`p-2 rounded-lg transition-colors shrink-0 ${
                            compactMode 
                                ? 'bg-blue-500/20 text-blue-400 hover:bg-blue-500/30' 
                                : 'bg-[var(--color-bg-tertiary)] text-[var(--color-text-secondary)] hover:bg-gray-700 hover:text-[var(--color-text-primary)]'
                        }`}
                        title={compactMode ? "Compact Mode: On" : "Compact Mode: Off"}
                    >
                        <Minimize2 size={16} />
                    </button>
                </div>
            </div>

            <div className="flex-1 overflow-y-auto p-2 space-y-1 custom-scrollbar">
                {/* Search Results Section */}
                {searchResults.length > 0 && (
                    <div className="mb-4">
                        <div className="px-2 text-xs font-bold text-[var(--color-text-tertiary)] uppercase tracking-wider mb-2">Message Matches</div>
                        {searchResults.map(res => (
                            <div
                                key={res.message_id}
                                onClick={() => onSelect(res.conversation_id)}
                                className="group w-full p-2.5 rounded-lg flex flex-col gap-1 cursor-pointer hover:bg-[var(--color-bg-tertiary)]/50 border border-transparent hover:border-[var(--color-border-secondary)]/50 transition-all mb-1"
                            >
                                <div className="flex items-center gap-2 text-[var(--color-text-secondary)] text-xs font-medium">
                                    <MessageSquare size={12} className="text-[var(--color-text-tertiary)]" />
                                    <span className="truncate">Match in Chat ...{res.conversation_id.slice(-4)}</span>
                                </div>
                                <div className="text-[var(--color-text-tertiary)] text-[10px] line-clamp-2">
                                    "{res.content}"
                                </div>
                            </div>
                        ))}
                        <div className="h-px bg-[var(--color-bg-tertiary)] my-2 mx-2" />
                    </div>
                )}

                {/* Conversations List */}
                {filteredConversations.length > 0 && searchResults.length > 0 && (
                    <div className="px-2 text-xs font-bold text-[var(--color-text-tertiary)] uppercase tracking-wider mb-2">Conversations</div>
                )}
                {filteredConversations.map(conv => (
                    <div
                        key={conv.id}
                        onMouseEnter={() => setHoveredId(conv.id)}
                        onMouseLeave={() => setHoveredId(null)}
                        onClick={() => onSelect(conv.id)}
                        className={`group relative w-full p-2.5 rounded-lg flex items-center gap-3 cursor-pointer transition-colors border border-transparent ${
                            compactMode ? 'h-auto' : 'h-14'
                        } ${activeId === conv.id
                            ? "bg-[var(--color-bg-tertiary)] border-[var(--color-border-secondary)]/50 shadow-sm"
                            : "hover:bg-[var(--color-bg-tertiary)]/50"
                            }`}
                    >
                        {conv.icon ? (
                            <span className="shrink-0 text-base leading-none" style={{ width: '16px', height: '16px', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                                {conv.icon}
                            </span>
                        ) : (
                            <MessageSquare size={16} className={`shrink-0 ${activeId === conv.id ? "text-blue-400" : "text-[var(--color-text-tertiary)] group-hover:text-[var(--color-text-secondary)]"}`} />
                        )}

                        <div className="flex-1 overflow-hidden min-w-0">
                            {editingId === conv.id ? (
                                <input
                                    autoFocus
                                    value={editTitle}
                                    onChange={(e) => setEditTitle(e.target.value)}
                                    // Removed onBlur for now to check behavior
                                    onKeyDown={(e) => e.key === 'Enter' && saveRename()}
                                    onClick={(e) => e.stopPropagation()}
                                    className="w-full bg-[var(--color-bg-primary)] text-[var(--color-text-primary)] text-sm px-1 py-0.5 rounded border border-blue-500 focus:outline-none"
                                />
                            ) : (
                                <>
                                    <div className={`font-medium text-sm truncate ${activeId === conv.id ? "text-gray-100" : "text-[var(--color-text-secondary)] group-hover:text-gray-200"}`}>
                                        {conv.title}
                                    </div>
                                    {!compactMode && (
                                        <div className="text-[10px] text-gray-600 truncate mt-0.5">{formatDate(conv.created_at)}</div>
                                    )}
                                </>
                            )}
                        </div>

                        {/* Actions (Icon/Rename/Delete).
                            Visibility is driven by the JS `hoveredId` state (set via
                            onMouseEnter/onMouseLeave) plus an active-row override.
                            Earlier versions also applied Tailwind `opacity-0
                            group-hover:opacity-100`, but the CSS `:hover` pseudo-class is
                            unreliable on WebKitGTK (Linux) — the buttons would render in
                            the DOM (clickable) yet stay invisible. JS state alone is
                            reliable across platforms. */}
                        {(hoveredId === conv.id || activeId === conv.id) && !editingId && (
                            <div className="absolute right-2 flex items-center gap-1 bg-[var(--color-bg-tertiary)] shadow-sm rounded-md p-0.5">
                                <button
                                    onClick={(e) => startEditIcon(e, conv)}
                                    className="p-1 hover:bg-gray-700 rounded text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]"
                                    title="Edit emoji"
                                >
                                    <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10" strokeWidth="2"/><path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M8 14s1.5 2 4 2 4-2 4-2M9 9h.01M15 9h.01"/></svg>
                                </button>
                                <button
                                    onClick={(e) => startRename(e, conv)}
                                    className="p-1 hover:bg-gray-700 rounded text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]"
                                    title="Rename"
                                >
                                    <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z"></path></svg>
                                </button>
                                <button
                                    onClick={(e) => handleDelete(e, conv)}
                                    className="p-1 hover:bg-red-900/50 rounded text-[var(--color-text-secondary)] hover:text-red-400"
                                    title="Delete"
                                >
                                    <Trash2 size={12} />
                                </button>
                            </div>
                        )}
                    </div>
                ))}

                {filteredConversations.length === 0 && !loading && (
                    <div className="text-center text-gray-600 text-sm mt-10">
                        {searchQuery ? "No matches found" : "No conversations yet"}
                    </div>
                )}
            </div>

            {/* Delete Confirmation Modal */}
            {deleteTarget && (
                <div className="fixed inset-0 bg-black/70 z-50 flex items-center justify-center p-4">
                    <div className="bg-[var(--color-bg-secondary)] border border-[var(--color-border-primary)] rounded-xl w-full max-w-md max-h-[80vh] flex flex-col shadow-2xl">
                        <div className="p-4 border-b border-[var(--color-border-primary)] flex items-center justify-between shrink-0">
                            <h4 className="font-bold text-[var(--color-text-primary)]">Delete chat?</h4>
                            <button
                                className="text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]"
                                onClick={() => setDeleteTarget(null)}
                                title="Close"
                            >
                                ×
                            </button>
                        </div>
                        <div className="p-4 text-sm text-[var(--color-text-secondary)] overflow-y-auto">
                            This will permanently delete <span className="font-semibold text-gray-100 break-words">{deleteTarget.title}</span> and all its messages.
                        </div>
                        <div className="p-4 border-t border-[var(--color-border-primary)] flex justify-end gap-2 shrink-0">
                            <button
                                className="px-4 py-2 rounded-lg hover:bg-[var(--color-bg-tertiary)] text-[var(--color-text-secondary)] font-semibold"
                                onClick={() => setDeleteTarget(null)}
                            >
                                Cancel
                            </button>
                            <button
                                className="px-4 py-2 rounded-lg bg-red-600 hover:bg-red-500 text-[var(--color-text-primary)] font-semibold"
                                onClick={confirmDelete}
                            >
                                Delete
                            </button>
                        </div>
                    </div>
                </div>
            )}

            {/* Icon Edit Modal */}
            {editingIconId && (
                <div className="fixed inset-0 bg-black/70 z-50 flex items-center justify-center p-4">
                    <div className="bg-[var(--color-bg-secondary)] border border-[var(--color-border-primary)] rounded-xl w-full max-w-md overflow-hidden shadow-2xl">
                        <div className="p-4 border-b border-[var(--color-border-primary)] flex items-center justify-between">
                            <h4 className="font-bold text-[var(--color-text-primary)]">Edit Chat Emoji</h4>
                            <button
                                className="text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]"
                                onClick={() => setEditingIconId(null)}
                                title="Close"
                            >
                                ×
                            </button>
                        </div>
                        <div className="p-4">
                            <label className="block text-sm font-medium text-[var(--color-text-secondary)] mb-2">
                                Enter an emoji (or leave empty for default icon)
                            </label>
                            <input
                                autoFocus
                                type="text"
                                value={editIcon}
                                onChange={(e) => setEditIcon(e.target.value)}
                                onKeyDown={(e) => e.key === 'Enter' && saveIcon()}
                                placeholder="🌟"
                                className="w-full bg-[var(--color-bg-primary)] text-[var(--color-text-primary)] text-2xl px-3 py-2 rounded-lg border border-[var(--color-border-secondary)] focus:outline-none focus:border-blue-500 text-center"
                                maxLength={2}
                            />
                            <p className="text-xs text-[var(--color-text-tertiary)] mt-2 text-center">
                                Paste or type any emoji character
                            </p>
                        </div>
                        <div className="p-4 border-t border-[var(--color-border-primary)] flex justify-end gap-2">
                            <button
                                className="px-4 py-2 rounded-lg hover:bg-[var(--color-bg-tertiary)] text-[var(--color-text-secondary)] font-semibold"
                                onClick={() => setEditingIconId(null)}
                            >
                                Cancel
                            </button>
                            <button
                                className="px-4 py-2 rounded-lg bg-blue-600 hover:bg-blue-500 text-white font-semibold"
                                onClick={saveIcon}
                            >
                                Save
                            </button>
                        </div>
                    </div>
                </div>
            )}

            {/* Resize Handle */}
            <div
                className={`absolute top-0 right-0 w-1 h-full cursor-col-resize hover:bg-blue-500/50 transition-colors ${
                    isResizing ? 'bg-blue-500' : ''
                }`}
                onMouseDown={(e) => {
                    e.preventDefault();
                    setIsResizing(true);
                }}
            />
        </div>
    );
}
