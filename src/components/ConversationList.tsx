
import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Plus, MessageSquare, Trash2, Search } from "lucide-react";

interface Conversation {
    id: string;
    title: string;
    created_at: string;
}

interface ConversationListProps {
    activeId: string | null;
    onSelect: (id: string) => void;
    onCreate: () => void;
}

export default function ConversationList({ activeId, onSelect, onCreate }: ConversationListProps) {
    const [conversations, setConversations] = useState<Conversation[]>([]);
    const [filteredConversations, setFilteredConversations] = useState<Conversation[]>([]);
    const [loading, setLoading] = useState(false);
    const [searchQuery, setSearchQuery] = useState("");

    // Rename/Delete State
    const [editingId, setEditingId] = useState<string | null>(null);
    const [editTitle, setEditTitle] = useState("");
    const [hoveredId, setHoveredId] = useState<string | null>(null);

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
        setFilteredConversations(
            conversations.filter(c => c.title.toLowerCase().includes(searchQuery.toLowerCase()))
        );
    }, [searchQuery, conversations]);

    const handleDelete = async (e: React.MouseEvent, id: string) => {
        e.stopPropagation();
        if (confirm("Are you sure you want to delete this chat?")) {
            try {
                await invoke("delete_conversation", { conversationId: id });

                // If we deleted the active conversation, decide what to select next
                if (activeId === id) {
                    const remaining = conversations.filter(c => c.id !== id);
                    if (remaining.length > 0) {
                        // Select the most recent one (first in list usually) or adjacent
                        // Since list is usually sorted by date desc, selecting 0 is fine
                        onSelect(remaining[0].id);
                    } else {
                        onCreate();
                    }
                }

                loadConversations();
            } catch (e) {
                console.error(e);
            }
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
            <div className="p-4 border-b border-gray-800 space-y-3">
                <button
                    onClick={onCreate}
                    className="w-full bg-blue-600 hover:bg-blue-500 text-white rounded-lg p-2.5 flex items-center justify-center gap-2 transition-all font-semibold text-sm shadow-md shadow-blue-900/20"
                >
                    <Plus size={18} /> New Chat
                </button>

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

            <div className="flex-1 overflow-y-auto p-2 space-y-1">
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
                                    onClick={(e) => handleDelete(e, conv.id)}
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
        </div>
    );
}
