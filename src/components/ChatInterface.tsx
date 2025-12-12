import { useState, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Send, Terminal, AlertTriangle } from "lucide-react";
import ReactMarkdown from "react-markdown";

interface ToolCall {
    id: string;
    function: {
        name: string;
        arguments: string;
    };
}

interface Message {
    role: "user" | "assistant" | "tool";
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

export default function ChatInterface({ conversationId }: ChatInterfaceProps) {
    const [messages, setMessages] = useState<Message[]>([]);
    const [input, setInput] = useState("");
    const [loading, setLoading] = useState(false);

    // Model Selection State
    const [availableModels, setAvailableModels] = useState<ModelOption[]>([]);
    const [selectedModel, setSelectedModel] = useState<string>(""); // stored as "providerId::modelId"

    const [pendingTool, setPendingTool] = useState<{ name: string, args: any, callId: string } | null>(null);
    const [errorMsg, setErrorMsg] = useState<string | null>(null);

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
        if (conversationId) {
            loadHistory(conversationId);
        } else {
            setMessages([]);
        }
    }, [conversationId]);

    useEffect(() => {
        scrollRef.current?.scrollIntoView({ behavior: "smooth" });
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
                // Default to first if not set or invalid
                if (!selectedModel || !models.find(m => `${m.providerId}::${m.id}` === selectedModel)) {
                    setSelectedModel(`${models[0].providerId}::${models[0].id}`);
                }
            }
        } catch (e) {
            console.error("Failed to load settings", e);
            setErrorMsg("Failed to load settings: " + String(e));
        }
    }

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

    return (
        <div className="flex flex-col h-screen bg-gray-950 text-white font-sans relative">
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
                </div>
            </div>

            <div className="flex-1 overflow-auto p-4 space-y-6">
                {messages.map((m, i) => (
                    <div
                        key={i}
                        className={`flex ${m.role === "user" ? "justify-end" : "justify-start"}`}
                    >
                        <div
                            className={`max-w-[80%] rounded-2xl p-4 ${m.role === "user"
                                ? "bg-blue-600 text-white rounded-br-none shadow-lg shadow-blue-900/20"
                                : m.role === "tool"
                                    ? "bg-gray-800 text-gray-400 font-mono text-xs rounded-none border-l-2 border-yellow-500 shadow-lg"
                                    : "bg-gray-800 text-gray-100 rounded-bl-none shadow-lg"
                                }`}
                        >
                            <div className="text-[10px] uppercase font-bold opacity-50 mb-1 flex items-center gap-1">
                                {m.role === "assistant" && "🤖 "}
                                {m.role === "user" && "👤 "}
                                {m.role}
                            </div>
                            {m.content && (
                                <div className="prose prose-invert prose-sm">
                                    <ReactMarkdown>{m.content}</ReactMarkdown>
                                </div>
                            )}
                            {m.tool_calls && (
                                <div className="mt-2 bg-gray-950 p-2 rounded border border-gray-700/50 font-mono text-xs">
                                    <div className="flex items-center gap-2 text-yellow-400">
                                        <Terminal size={14} />
                                        <span>Tool Call: {m.tool_calls[0]?.function?.name}</span>
                                    </div>
                                </div>
                            )}
                        </div>
                    </div>
                ))}
                <div ref={scrollRef} />
            </div>

            {pendingTool && (
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
            )}

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
        </div>
    );
}
