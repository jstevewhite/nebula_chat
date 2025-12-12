import { useState, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Send, Terminal, AlertTriangle, Copy, Edit2, Trash2, RefreshCw, Check, Pin, FileText, Book } from "lucide-react";
import ReactMarkdown from "react-markdown";
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism';
import remarkGfm from 'remark-gfm';

interface ToolCall {
    id: string;
    function: {
        name: string;
        arguments: string;
    };
}

interface Message {
    id?: string;
    role: "user" | "assistant" | "system" | "tool";
    content: string | null;
    tool_calls?: ToolCall[];
    tool_call_id?: string;
}

interface ChatInterfaceProps {
    conversationId: string | null;
}

interface ModelOption {
    id: string;
    name: string;
    providerId: string;
}

interface SystemPrompt {
    id: string;
    name: string;
    content: string;
}

export default function ChatInterface({ conversationId }: ChatInterfaceProps) {
    const [messages, setMessages] = useState<Message[]>([]);
    const [input, setInput] = useState("");
    const [loading, setLoading] = useState(false);

    // Model Selection State
    const [availableModels, setAvailableModels] = useState<ModelOption[]>([]);
    const [selectedModel, setSelectedModel] = useState<string>(""); // stored as "providerId::modelId"

    // System Prompts
    const [prompts, setPrompts] = useState<SystemPrompt[]>([]);
    const [selectedPromptId, setSelectedPromptId] = useState<string>("");

    const [pendingTool, setPendingTool] = useState<{ name: string, args: any, callId: string } | null>(null);
    const [errorMsg, setErrorMsg] = useState<string | null>(null);
    const [copiedCodeVal, setCopiedCodeVal] = useState<string | null>(null); // To show 'Check' icon momentarily

    const scrollRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        loadSettings();
    }, []);

    // Auto-clear error after 5s
    useEffect(() => {
        if (errorMsg) {
            const timer = setTimeout(() => setErrorMsg(null), 5000);
            return () => clearTimeout(timer);
        }
    }, [errorMsg]);

    useEffect(() => {
        if (copiedCodeVal) {
            const timer = setTimeout(() => setCopiedCodeVal(null), 2000);
            return () => clearTimeout(timer);
        }
    }, [copiedCodeVal]);

    useEffect(() => {
        if (conversationId) {
            loadHistory(conversationId);
        } else {
            setMessages([]);
        }
    }, [conversationId]);

    useEffect(() => {
        if (scrollRef.current) {
            const container = scrollRef.current;
            // Use setTimeout to allow layout to settle (e.g. images loading, markdown rendering)
            const timer = setTimeout(() => {
                container.scrollTo({
                    top: container.scrollHeight,
                    behavior: "smooth"
                });
            }, 100);
            return () => clearTimeout(timer);
        }
    }, [messages]);

    const loadHistory = async (id: string) => {
        try {
            const history = await invoke<Message[]>("get_chat_history", { conversationId: id });
            setMessages(history);
        } catch (e) {
            console.error("Failed to load history", e);
        }
    }

    const loadSettings = async () => {
        try {
            const settings: any = await invoke("get_settings");
            const models: ModelOption[] = [];

            if (settings.providers) {
                Object.entries(settings.providers).forEach(([providerKey, config]: [string, any]) => {
                    if (config.enabled && config.models) {
                        config.models.forEach((m: any) => {
                            if (m.visible !== false) {
                                models.push({
                                    id: m.id,
                                    name: m.name || m.id,
                                    providerId: providerKey
                                });
                            }
                        });
                    }
                });
            }

            setAvailableModels(models);

            if (models.length > 0) {
                // Priority: State (if already set) > Default Setting > First Available
                if (!selectedModel) {
                    if (settings.default_model && models.find(m => `${m.providerId}::${m.id}` === settings.default_model)) {
                        setSelectedModel(settings.default_model);
                    } else {
                        setSelectedModel(`${models[0].providerId}::${models[0].id}`);
                    }
                }
            }

            // Load Prompts
            const promptsList = await invoke<SystemPrompt[]>("get_system_prompts");
            setPrompts(promptsList);
            if (settings.active_system_prompt_id) {
                setSelectedPromptId(settings.active_system_prompt_id);
            }
        } catch (e) {
            console.error("Failed to load settings", e);
            setErrorMsg("Failed to load settings: " + String(e));
        }
    }

    const handleSetPrompt = async (id: string) => {
        setSelectedPromptId(id);
        await invoke("set_active_system_prompt", { id: id || null });
    };

    const handleSetDefaultModel = async () => {
        if (!selectedModel) return;
        try {
            await invoke("set_default_model", { modelTarget: selectedModel });
            // Optional: Show success feedback
            const btn = document.getElementById("pin-model-btn");
            if (btn) {
                btn.classList.add("text-green-400");
                setTimeout(() => btn.classList.remove("text-green-400"), 1000);
            }
        } catch (e) {
            setErrorMsg("Failed to set default model: " + String(e));
        }
    };

    const sendMessage = async (currentHistory: Message[]) => {
        setLoading(true);
        setErrorMsg(null);
        try {
            if (!selectedModel) throw new Error("No model selected or available.");

            const [providerId, modelId] = selectedModel.split("::");

            const response = await invoke<Message>("send_message", {
                messages: currentHistory,
                providerId: providerId,  // Changed from apiKey
                model: modelId,
                conversationId: conversationId
            });

            const newHistory = [...currentHistory, response];
            setMessages(newHistory);

            // Auto-Title Trigger (If this was the first exchange)
            if (currentHistory.length === 1 && conversationId) {
                // Async background trigger, don't await
                invoke("generate_title", {
                    conversationId,
                    providerId,
                    model: modelId
                }).catch(e => console.error("Auto-title failed", e));
            }

            if (response.tool_calls && response.tool_calls.length > 0) {
                const toolCall = response.tool_calls[0];
                try {
                    const args = JSON.parse(toolCall.function.arguments);
                    setPendingTool({
                        name: toolCall.function.name,
                        args: args,
                        callId: toolCall.id || "call_" + Math.random().toString(36).substr(2, 9)
                    });
                } catch (e) {
                    console.error("Failed to parse tool args", e);
                }
                setLoading(false); // Paused for user
            } else {
                setLoading(false);
            }
        } catch (error) {
            console.error(error);
            setLoading(false);
            setErrorMsg(String(error));
        }
    };

    const handleSend = async () => {
        if (!input.trim()) return;
        const userMsg: Message = { role: "user", content: input };
        const newHistory = [...messages, userMsg];
        setMessages(newHistory);
        setInput("");
        await sendMessage(newHistory);
    };

    const handleApproveTool = async () => {
        if (!pendingTool) return;
        setLoading(true);
        try {
            const result = await invoke("execute_tool", {
                name: pendingTool.name,
                args: pendingTool.args
            });

            const toolMsg: Message = {
                role: "tool",
                content: JSON.stringify(result),
                tool_call_id: pendingTool.callId
            };

            const newHistory = [...messages, toolMsg];
            setMessages(newHistory);
            setPendingTool(null);

            // Continue conversation
            await sendMessage(newHistory);
        } catch (e) {
            console.error(e);
            setLoading(false);
            setPendingTool(null); // Or show error
            setErrorMsg("Tool Execution Failed: " + String(e));
        }
    };

    const handleCopy = (text: string) => {
        navigator.clipboard.writeText(text);
    };



    const handleDelete = async (index: number, id?: string) => {
        if (id) {
            try {
                await invoke("delete_message", { messageId: id });
            } catch (e) {
                console.error("Failed to delete message", e);
            }
        }
        const newMsgs = [...messages];
        newMsgs.splice(index, 1);
        setMessages(newMsgs);
    };

    const handleRegenerate = async (index: number, id?: string) => {
        // 1. Delete the assistant message
        await handleDelete(index, id);

        // 2. Capture history up to the point before this message
        // The last message in this slice should be the User message we want to re-run
        // NOTE: We assume the existing history is correct.
        const historyToReplay = messages.slice(0, index);

        // 3. Trigger send
        await sendMessage(historyToReplay);
    };

    const handleEdit = (content: string) => {
        setInput(content);
        // Optionally focus input?
    };

    return (
        <div className="flex flex-col h-full bg-gray-950 text-white font-sans relative">
            {errorMsg && (
                <div className="absolute top-4 left-1/2 transform -translate-x-1/2 z-50 bg-red-600/90 backdrop-blur text-white px-4 py-3 rounded-xl shadow-2xl animate-fade-in-down flex items-center gap-3 border border-red-500/50">
                    <AlertTriangle size={20} className="text-white" />
                    <span className="font-semibold">{errorMsg}</span>
                </div>
            )}

            <div className="p-4 bg-gray-900 border-b border-gray-800 flex justify-between items-center shadow-md z-10">
                <div className="flex items-center gap-3">
                    <div className="w-3 h-3 rounded-full bg-green-500 animate-pulse box-shadow-lg shadow-green-500/50" />
                    <select
                        value={selectedModel}
                        onChange={(e) => setSelectedModel(e.target.value)}
                        className="bg-gray-800 text-white text-sm rounded-lg border border-gray-700 focus:ring-blue-500 focus:border-blue-500 block p-2.5 font-medium max-w-[200px]"
                        style={{ colorScheme: "dark" }}
                    >
                        {availableModels.length === 0 && <option disabled>No enabled models</option>}
                        {availableModels.map(m => (
                            <option key={`${m.providerId}::${m.id}`} value={`${m.providerId}::${m.id}`}>
                                {m.providerId === "ollama" ? `🦙 ${m.name}` :
                                    m.providerId === "openai" ? `🤖 ${m.name}` :
                                        m.providerId === "anthropic" ? `🧠 ${m.name}` :
                                            `${m.name}`}
                            </option>
                        ))}
                    </select>

                    {/* Prompt Selector */}
                    <div className="flex items-center gap-1 border-l border-gray-700 pl-3">
                        <Book size={16} className="text-gray-400" />
                        <select
                            value={selectedPromptId}
                            onChange={(e) => handleSetPrompt(e.target.value)}
                            className="bg-gray-800 text-white text-sm rounded-lg border border-gray-700 focus:ring-blue-500 focus:border-blue-500 block p-2.5 font-medium max-w-[150px]"
                            style={{ colorScheme: "dark" }}
                        >
                            <option value="">Default System</option>
                            {prompts.map(p => (
                                <option key={p.id} value={p.id}>{p.name}</option>
                            ))}
                        </select>
                    </div>
                    <button
                        id="pin-model-btn"
                        onClick={handleSetDefaultModel}
                        className="p-2 text-gray-400 hover:text-white transition-colors"
                        title="Set as Default Model"
                    >
                        <Pin size={16} />
                    </button>
                </div>
            </div>

            <div
                className="flex-1 overflow-y-auto p-4 space-y-6 min-h-0"
                ref={scrollRef}
            >
                {messages.map((m, i) => (
                    <ChatMessage
                        key={i}
                        message={m}
                        index={i}
                        onCopy={handleCopy}
                        onEdit={handleEdit}
                        onDelete={handleDelete}
                        onRegenerate={handleRegenerate}
                    />
                ))}
            </div>

            {
                pendingTool && (
                    <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4 z-50 animate-in fade-in duration-200">
                        <div className="bg-gray-900 p-6 rounded-xl max-w-lg w-full border border-gray-700 shadow-2xl ring-1 ring-white/10">
                            <h3 className="text-lg font-bold mb-4 flex items-center gap-2 text-white">
                                <Terminal className="text-yellow-400" /> Use Tool?
                            </h3>
                            <div className="bg-black/50 p-4 rounded-lg mb-6 font-mono text-sm overflow-auto max-h-60 border border-white/5">
                                <p className="text-green-400 font-bold mb-2">$ {pendingTool.name}</p>
                                <pre className="text-gray-400 whitespace-pre-wrap break-all">
                                    {JSON.stringify(pendingTool.args, null, 2)}
                                </pre>
                            </div>
                            <div className="flex justify-end gap-3">
                                <button
                                    onClick={() => setPendingTool(null)}
                                    className="px-4 py-2 rounded-lg bg-gray-800 hover:bg-gray-700 text-gray-300 transition-colors"
                                >
                                    Deny
                                </button>
                                <button
                                    onClick={handleApproveTool}
                                    className="px-4 py-2 rounded-lg bg-blue-600 hover:bg-blue-500 text-white font-semibold shadow-lg shadow-blue-500/20 transition-all hover:scale-105"
                                >
                                    Allow Execution
                                </button>
                            </div>
                        </div>
                    </div>
                )
            }


            <div className="p-4 bg-gray-900 border-t border-gray-800">
                <div className="max-w-4xl mx-auto flex gap-3">
                    <input
                        className="flex-1 bg-gray-800 border border-gray-700 rounded-xl p-3 text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent transition-all"
                        value={input}
                        onChange={(e) => setInput(e.target.value)}
                        onKeyDown={(e) => e.key === "Enter" && !e.shiftKey && handleSend()}
                        placeholder="Type a message..."
                        disabled={loading}
                    />
                    <button
                        disabled={loading}
                        onClick={handleSend}
                        className="bg-blue-600 p-3 rounded-xl hover:bg-blue-500 disabled:opacity-50 disabled:cursor-not-allowed transition-colors shadow-lg shadow-blue-600/20"
                    >
                        {loading ? <div className="w-5 h-5 border-2 border-white/30 border-t-white rounded-full animate-spin" /> : <Send size={20} />}
                    </button>
                </div>
            </div>
        </div >
    );
}





function ChatMessage({ message: m, index: i, onCopy, onEdit, onDelete, onRegenerate }: any) {
    const [isExpanded, setIsExpanded] = useState(m.role !== "tool");
    const [showRaw, setShowRaw] = useState(false);
    const [showToolArgs, setShowToolArgs] = useState(false);
    const [copiedCodeVal, setCopiedCodeVal] = useState<string | null>(null);

    useEffect(() => {
        if (copiedCodeVal) {
            const timer = setTimeout(() => setCopiedCodeVal(null), 2000);
            return () => clearTimeout(timer);
        }
    }, [copiedCodeVal]);

    const handleCopyCode = (text: string) => {
        navigator.clipboard.writeText(text);
        setCopiedCodeVal(text);
    };

    return (
        <div className={`flex ${m.role === "user" ? "justify-end" : "justify-start"}`}>
            <div
                className={`max-w-[85%] rounded-xl p-4 relative group ${m.role === "user"
                    ? "bg-blue-600 text-white rounded-br-none shadow-lg shadow-blue-900/20"
                    : m.role === "tool"
                        ? "bg-gray-800 text-gray-400 font-mono text-xs rounded-none border-l-2 border-yellow-500 shadow-lg"
                        : "bg-[#1e1e1e] text-gray-100 rounded-bl-none shadow-lg border border-white/5"
                    }`}
            >
                <div className="flex justify-between items-start gap-4 mb-2">
                    <div className="text-[10px] uppercase font-bold opacity-50 flex items-center gap-1 select-none">
                        {m.role === "assistant" && "🤖 Assistant"}
                        {m.role === "user" && "👤 You"}
                        {m.role === "tool" && "🛠 Tool Output"}
                        {m.role === "system" && "⚙ System"}
                    </div>

                    <div className={`flex gap-2 opacity-0 group-hover:opacity-100 transition-opacity ${m.role === "user" ? "text-blue-200" : "text-gray-400"}`}>
                        <button
                            onClick={() => setShowRaw(!showRaw)}
                            className={`hover:text-white ${showRaw ? "text-blue-400" : ""}`}
                            title="Toggle Raw View"
                        >
                            <FileText size={13} />
                        </button>

                        <button onClick={() => onCopy(m.content || "")} className="hover:text-white" title="Copy Message">
                            <Copy size={13} />
                        </button>

                        {m.role === "user" && (
                            <button onClick={() => onEdit(m.content || "")} className="hover:text-white" title="Edit">
                                <Edit2 size={13} />
                            </button>
                        )}

                        {(m.role === "assistant" || m.role === "user") && (
                            <button onClick={() => onDelete(i, m.id)} className="hover:text-red-400" title="Delete">
                                <Trash2 size={13} />
                            </button>
                        )}

                        {m.role === "assistant" && (
                            <button onClick={() => onRegenerate(i, m.id)} className="hover:text-green-400" title="Regenerate">
                                <RefreshCw size={13} />
                            </button>
                        )}
                    </div>
                </div>

                {/* Collapsible Content */}
                {m.role === "tool" && !isExpanded ? (
                    <div className="flex items-center gap-3 mt-2">
                        <button
                            onClick={() => setIsExpanded(true)}
                            className="text-blue-400 hover:text-blue-300 underline flex items-center gap-1"
                        >
                            Show Output ({m.content?.length || 0} chars)
                        </button>
                    </div>
                ) : (
                    <>
                        {showRaw ? (
                            <pre className="whitespace-pre-wrap font-mono text-sm text-gray-300 bg-black/20 p-2 rounded border border-white/10 overflow-x-auto">
                                {m.content}
                            </pre>
                        ) : (
                            m.content && (
                                <div className={`prose max-w-none ${m.role === "user" ? "prose-invert text-white prose-pre:bg-blue-800" : "prose-invert text-gray-300"}`}>
                                    <ReactMarkdown
                                        remarkPlugins={[remarkGfm]}
                                        components={{
                                            ul: ({ node, ...props }) => <ul className="list-disc pl-6 mb-4 space-y-2" {...props} />,
                                            ol: ({ node, ...props }) => <ol className="list-decimal pl-6 mb-4 space-y-2" {...props} />,
                                            li: ({ node, ...props }) => <li className="leading-relaxed pl-1" {...props} />,
                                            h1: ({ node, ...props }) => <h1 className="text-2xl font-bold mb-4 mt-6 text-white pb-2 border-b border-gray-700" {...props} />,
                                            h2: ({ node, ...props }) => <h2 className="text-xl font-bold mb-3 mt-5 text-gray-100" {...props} />,
                                            h3: ({ node, ...props }) => <h3 className="text-lg font-semibold mb-2 mt-4 text-gray-200" {...props} />,
                                            p: ({ node, ...props }) => <p className="leading-7 mb-4 text-gray-300" {...props} />,
                                            strong: ({ node, ...props }) => <strong className="font-bold text-white" {...props} />,
                                            a: ({ node, ...props }) => <a className="text-blue-400 hover:underline" target="_blank" rel="noopener noreferrer" {...props} />,
                                            blockquote: ({ node, ...props }) => <blockquote className="border-l-4 border-gray-600 pl-4 italic text-gray-400 my-4" {...props} />,

                                            code({ node, inline, className, children, ...props }: any) {
                                                const match = /language-(\w+)/.exec(className || '')
                                                const codeText = String(children).replace(/\n$/, '')
                                                const isCopied = copiedCodeVal === codeText

                                                return !inline && match ? (
                                                    <div className="relative group/code my-4">
                                                        <div className="absolute top-2 right-2 opacity-0 group-hover/code:opacity-100 transition-opacity z-10">
                                                            <button
                                                                onClick={() => handleCopyCode(codeText)}
                                                                className="p-1.5 bg-gray-700/80 hover:bg-gray-600 rounded text-gray-300 transition-colors backdrop-blur-sm"
                                                                title="Copy Code"
                                                            >
                                                                {isCopied ? <Check size={14} className="text-green-400" /> : <Copy size={14} />}
                                                            </button>
                                                        </div>
                                                        <SyntaxHighlighter
                                                            // @ts-ignore
                                                            style={vscDarkPlus}
                                                            language={match[1]}
                                                            PreTag="div"
                                                            customStyle={{ margin: 0, borderRadius: '0.5rem', background: '#0d1117', fontSize: '14px' }}
                                                            {...props}
                                                        >
                                                            {codeText}
                                                        </SyntaxHighlighter>
                                                    </div>
                                                ) : (
                                                    <code className={`${className} bg-white/10 px-1.5 py-0.5 rounded text-[0.9em] font-mono text-yellow-200`} {...props}>
                                                        {children}
                                                    </code>
                                                )
                                            }
                                        }}
                                    >
                                        {m.content}
                                    </ReactMarkdown>
                                </div>
                            )
                        )}
                        {m.role === "tool" && (
                            <button
                                onClick={() => setIsExpanded(false)}
                                className="text-gray-500 hover:text-gray-400 text-xs mt-2 underline"
                            >
                                Collapse Output
                            </button>
                        )}
                    </>
                )}

                {m.tool_calls && (
                    <div className="mt-2 text-xs">
                        <button
                            onClick={() => setShowToolArgs(!showToolArgs)}
                            className="bg-gray-950 p-2 rounded border border-gray-700/50 hover:border-gray-500 transition-colors w-full text-left"
                        >
                            <div className="flex items-center gap-2 text-yellow-400">
                                <Terminal size={14} />
                                <span className="font-mono">Tool Call: {m.tool_calls[0]?.function?.name}</span>
                                <span className="text-gray-500 ml-auto text-[10px]">{showToolArgs ? "Hide Args" : "Show Args"}</span>
                            </div>
                        </button>

                        {showToolArgs && (
                            <div className="mt-1 bg-black/40 p-2 rounded border-x border-b border-gray-800 font-mono overflow-x-auto animate-in fade-in zoom-in-95 duration-200">
                                <pre className="text-gray-400 whitespace-pre-wrap break-all">
                                    {m.tool_calls[0]?.function?.arguments}
                                </pre>
                            </div>
                        )}
                    </div>
                )}
            </div>
        </div>
    );
}
