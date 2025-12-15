
import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Plus, MessageSquare, Trash2, Search, Upload } from "lucide-react";

interface Conversation {
    id: string;
    title: string;
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

    const formatDate = (dateStr: string) => {
        const date = new Date(dateStr);
        return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    }

    return (
        <div className="w-64 bg-gray-900 border-r border-gray-800 flex flex-col h-full select-none">
            <input type="file" ref={importRef} onChange={handleImport} className="hidden" accept=".json" />
            <div className="p-4 border-b border-gray-800 space-y-3">
                {error && (
                    <div className="text-xs bg-red-900/20 border border-red-700/40 text-red-200 rounded-lg px-3 py-2">
                        {error}
                    </div>
                )}
                <div className="flex gap-2">
                    <button
                        onClick={onCreate}
                        className="flex-1 bg-blue-600 hover:bg-blue-500 text-white rounded-lg p-2.5 flex items-center justify-center gap-2 transition-all font-semibold text-sm shadow-md shadow-blue-900/20"
                    >
                        <Plus size={18} /> New Chat
                    </button>
                    <button
                        onClick={() => importRef.current?.click()}
                        className="bg-gray-800 hover:bg-gray-700 text-gray-400 hover:text-white rounded-lg p-2.5 flex items-center justify-center transition-all shadow-md"
                        title="Import JSON"
                    >
                        <Upload size={18} />
                    </button>
                </div>

                <div className="relative">
                    <Search className="absolute left-2.5 top-2.5 text-gray-500 w-4 h-4" />
                    <input
                        type="text"
                        placeholder="Search chats..."
                        value={searchQuery}
                        onChange={(e) => setSearchQuery(e.target.value)}
                        className="w-full bg-gray-950 border border-gray-800 rounded-lg py-2 pl-9 pr-3 text-sm text-gray-200 focus:outline-none focus:border-blue-500/50 transition-colors placeholder-gray-600"
                    />
                </div>
            </div>

            <div className="flex-1 overflow-y-auto p-2 space-y-1 custom-scrollbar">
                {/* Search Results Section */}
                {searchResults.length > 0 && (
                    <div className="mb-4">
                        <div className="px-2 text-xs font-bold text-gray-500 uppercase tracking-wider mb-2">Message Matches</div>
                        {searchResults.map(res => (
                            <div
                                key={res.message_id}
                                onClick={() => onSelect(res.conversation_id)}
                                className="group w-full p-2.5 rounded-lg flex flex-col gap-1 cursor-pointer hover:bg-gray-800/50 border border-transparent hover:border-gray-700/50 transition-all mb-1"
                            >
                                <div className="flex items-center gap-2 text-gray-300 text-xs font-medium">
                                    <MessageSquare size={12} className="text-gray-500" />
                                    <span className="truncate">Match in Chat ...{res.conversation_id.slice(-4)}</span>
                                </div>
                                <div className="text-gray-500 text-[10px] line-clamp-2">
                                    "{res.content}"
                                </div>
                            </div>
                        ))}
                        <div className="h-px bg-gray-800 my-2 mx-2" />
                    </div>
                )}

                {/* Conversations List */}
                {filteredConversations.length > 0 && searchResults.length > 0 && (
                    <div className="px-2 text-xs font-bold text-gray-500 uppercase tracking-wider mb-2">Conversations</div>
                )}
                {filteredConversations.map(conv => (
                    <div
                        key={conv.id}
                        onMouseEnter={() => setHoveredId(conv.id)}
                        onMouseLeave={() => setHoveredId(null)}
                        onClick={() => onSelect(conv.id)}
                        className={`group relative w-full h-14 p-2.5 rounded-lg flex items-center gap-3 cursor-pointer transition-colors border border-transparent ${activeId === conv.id
                            ? "bg-gray-800 border-gray-700/50 shadow-sm"
                            : "hover:bg-gray-800/50"
                            }`}
                    >
                        <MessageSquare size={16} className={`shrink-0 ${activeId === conv.id ? "text-blue-400" : "text-gray-500 group-hover:text-gray-400"}`} />

                        <div className="flex-1 overflow-hidden min-w-0">
                            {editingId === conv.id ? (
                                <input
                                    autoFocus
                                    value={editTitle}
                                    onChange={(e) => setEditTitle(e.target.value)}
                                    // Removed onBlur for now to check behavior
                                    onKeyDown={(e) => e.key === 'Enter' && saveRename()}
                                    onClick={(e) => e.stopPropagation()}
                                    className="w-full bg-gray-950 text-white text-sm px-1 py-0.5 rounded border border-blue-500 focus:outline-none"
                                />
                            ) : (
                                <>
                                    <div className={`font-medium text-sm truncate ${activeId === conv.id ? "text-gray-100" : "text-gray-400 group-hover:text-gray-200"}`}>
                                        {conv.title}
                                    </div>
                                    <div className="text-[10px] text-gray-600 truncate mt-0.5">{formatDate(conv.created_at)}</div>
                                </>
                            )}
                        </div>

                        {/* Actions (Rename/Delete) */}
                        {(hoveredId === conv.id || activeId === conv.id) && !editingId && (
                            <div className="absolute right-2 flex items-center gap-1 bg-gray-800 shadow-sm rounded-md p-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                                <button
                                    onClick={(e) => startRename(e, conv)}
                                    className="p-1 hover:bg-gray-700 rounded text-gray-400 hover:text-white"
                                    title="Rename"
                                >
                                    <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z"></path></svg>
                                </button>
                                <button
                                    onClick={(e) => handleDelete(e, conv)}
                                    className="p-1 hover:bg-red-900/50 rounded text-gray-400 hover:text-red-400"
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
                    <div className="bg-gray-900 border border-gray-800 rounded-xl w-full max-w-md overflow-hidden shadow-2xl">
                        <div className="p-4 border-b border-gray-800 flex items-center justify-between">
                            <h4 className="font-bold text-white">Delete chat?</h4>
                            <button
                                className="text-gray-400 hover:text-white"
                                onClick={() => setDeleteTarget(null)}
                                title="Close"
                            >
                                ×
                            </button>
                        </div>
                        <div className="p-4 text-sm text-gray-300">
                            This will permanently delete <span className="font-semibold text-gray-100">{deleteTarget.title}</span> and all its messages.
                        </div>
                        <div className="p-4 border-t border-gray-800 flex justify-end gap-2">
                            <button
                                className="px-4 py-2 rounded-lg hover:bg-gray-800 text-gray-300 font-semibold"
                                onClick={() => setDeleteTarget(null)}
                            >
                                Cancel
                            </button>
                            <button
                                className="px-4 py-2 rounded-lg bg-red-600 hover:bg-red-500 text-white font-semibold"
                                onClick={confirmDelete}
                            >
                                Delete
                            </button>
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
}
