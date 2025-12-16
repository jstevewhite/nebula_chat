import { useState, useRef, useEffect } from "react";
import { flushSync } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Send, Terminal, AlertTriangle, Copy, Edit2, Trash2, RefreshCw, Check, Pin, FileText, Book, Paperclip, X, Brain, Square, Sliders, Download } from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile, readFile, readTextFile } from "@tauri-apps/plugin-fs";
import ReactMarkdown from "react-markdown";
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism';
import remarkGfm from 'remark-gfm';
import MemoryPanel from "./MemoryPanel";
import { getProviderIcon } from "../utils/providerIcons";
import { useTheme } from "../contexts/ThemeContext";
import { CustomSelect } from "./ui/CustomSelect";

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
    attachments?: {
        name: string;
        media_type: string;
        data: string;
        is_binary: boolean;
    }[];
}

interface ChatInterfaceProps {
    conversationId: string | null;
}

interface ModelOption {
    id: string;
    name: string;
    providerId: string;
    providerType: string;
}

interface SystemPrompt {
    id: string;
    name: string;
    content: string;
}

interface GenerationSettings {
    temperature: number;
    top_p: number;
    stream: boolean;
}

export default function ChatInterface({ conversationId }: ChatInterfaceProps) {
    const { theme } = useTheme();

    const [messages, setMessages] = useState<Message[]>([]);
    const [input, setInput] = useState("");
    const [loading, setLoading] = useState(false);

    const [isDragging, setIsDragging] = useState(false);



    // Model Selection State
    const [availableModels, setAvailableModels] = useState<ModelOption[]>([]);
    const [selectedModel, setSelectedModel] = useState<string>(""); // stored as "providerId::modelId"

    // Generation Settings
    const [genSettings, setGenSettings] = useState<GenerationSettings>({
        temperature: 0.7,
        top_p: 1.0,
        stream: true
    });
    const [showSettings, setShowSettings] = useState(false);

    interface Attachment {
        file: File;
        preview: string;
        base64: string;
        isBinary: boolean;
        mediaType: string;
    }
    const [attachments, setAttachments] = useState<Attachment[]>([]);

    // System Prompts
    const [prompts, setPrompts] = useState<SystemPrompt[]>([]);
    const [selectedPromptId, setSelectedPromptId] = useState<string>("");

    // Side Panels
    const [activeSidePanel, setActiveSidePanel] = useState<'none' | 'memory'>('none');
    const [recentMemories, setRecentMemories] = useState<string[]>([]);

    // Tool Execution State
    const [pendingTools, setPendingTools] = useState<{ name: string, args: any, callId: string }[]>([]);
    const [toolPolicies, setToolPolicies] = useState<Record<string, boolean>>({});

    // Listen for Memory Events
    useEffect(() => {
        const unlistenPromise = listen<string[]>("memory-context", (event) => {
            console.log("Memory Context Received:", event.payload);
            setRecentMemories(event.payload);
        });
        return () => {
            unlistenPromise.then(unlisten => unlisten());
        };
    }, []);
    const [errorMsg, setErrorMsg] = useState<string | null>(null);
    const [copiedCodeVal, setCopiedCodeVal] = useState<string | null>(null); // To show 'Check' icon momentarily

    const scrollRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        loadSettings();

        const unlistenPromise = listen("tools-updated", () => {
            loadSettings();
        });

        return () => {
            unlistenPromise.then(unlisten => unlisten());
        };
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

    // Tauri File Drop Listeners
    useEffect(() => {
        let unlistenHover: (() => void) | Promise<(() => void)>;
        let unlistenDrop: (() => void) | Promise<(() => void)>;
        let unlistenCancelled: (() => void) | Promise<(() => void)>;

        const setupListeners = async () => {
            console.log("Setting up Tauri File Drop listeners...");
            unlistenHover = listen("tauri://file-drop-hover", () => {
                console.log("Event: tauri://file-drop-hover");
                setIsDragging(true);
            });

            unlistenDrop = listen<string[]>("tauri://file-drop", async (event) => {
                console.log("Event: tauri://file-drop", event);
                setIsDragging(false);
                if (event.payload && event.payload.length > 0) {
                    await processDroppedPaths(event.payload);
                }
            });

            unlistenCancelled = listen("tauri://file-drop-cancelled", () => {
                console.log("Event: tauri://file-drop-cancelled");
                setIsDragging(false);
            });

            console.log("Listeners initiated.");
        };

        setupListeners();

        return () => {
            if (unlistenHover) Promise.resolve(unlistenHover).then(u => u());
            if (unlistenDrop) Promise.resolve(unlistenDrop).then(u => u());
            if (unlistenCancelled) Promise.resolve(unlistenCancelled).then(u => u());
        };
    }, []);

    function uint8ArrayToBase64(bytes: Uint8Array): string {
        let binary = '';
        const len = bytes.byteLength;
        for (let i = 0; i < len; i++) {
            binary += String.fromCharCode(bytes[i]);
        }
        return window.btoa(binary);
    }

    // Process paths from Tauri Drop
    const processDroppedPaths = async (paths: string[]) => {
        console.log("Processing dropped paths:", paths);
        for (const path of paths) {
            try {
                // Heuristic for mime type based on extension
                const name = path.split(/[\\/]/).pop() || "unknown";
                const ext = name.split('.').pop()?.toLowerCase();

                let isImage = ['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp'].includes(ext || "");
                let content = "";
                let preview = "";
                let mediaType = "application/octet-stream";

                if (isImage) {
                    const data = await readFile(path);
                    const base64 = uint8ArrayToBase64(data);

                    if (ext === 'png') mediaType = 'image/png';
                    else if (ext === 'jpg' || ext === 'jpeg') mediaType = 'image/jpeg';
                    else if (ext === 'webp') mediaType = 'image/webp';
                    else if (ext === 'gif') mediaType = 'image/gif';

                    content = `data:${mediaType};base64,${base64}`;
                    preview = content;
                } else {
                    // Assume text for other files for now, or read as text
                    const text = await readTextFile(path);
                    content = text;
                    preview = "TEXT_FILE";
                    mediaType = "text/plain";
                    // If we wanted binary non-image, we'd need generic handling. 
                    // For now, assume text unless image.
                }

                setAttachments(prev => [...prev, {
                    file: new File([""], name, { type: mediaType }), // Dummy File object for interface compatibility
                    preview,
                    base64: content, // Data URL or Text
                    isBinary: isImage,
                    mediaType: mediaType
                }]);
            } catch (e) {
                console.error("Failed to read file", path, e);
                setErrorMsg(`Failed to read file ${path}: ${String(e)}`);
            }
        }
    };

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
                                    providerId: providerKey,
                                    providerType: config.provider_type // Extract provider type
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

            // Load Tool Policies
            const policies = await invoke<Record<string, boolean>>("get_tool_policies");
            setToolPolicies(policies);
        } catch (e) {
            console.error("Failed to fetch models", e);
            setErrorMsg("Failed to fetch models: " + String(e));
        }
    };

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

    const handleFileSelect = async (e: React.ChangeEvent<HTMLInputElement>) => {
        if (e.target.files) {
            processFiles(Array.from(e.target.files));
        }
    };

    const processFiles = (files: File[]) => {
        console.log("Processing dropped files:", files.length);
        for (const file of files) {
            console.log("File:", file.name, file.type, file.size);
            const isImage = file.type.startsWith('image/');
            const reader = new FileReader();

            reader.onload = (ev) => {
                if (ev.target?.result) {
                    let content = ev.target.result as string;
                    let preview = "";

                    if (isImage) {
                        preview = content;
                    } else {
                        preview = "TEXT_FILE";
                    }

                    setAttachments(prev => [...prev, {
                        file,
                        preview,
                        base64: content, // Data URL
                        isBinary: isImage,
                        mediaType: file.type || "text/plain"
                    }]);
                }
            };

            if (isImage) {
                reader.readAsDataURL(file);
            } else {
                reader.readAsText(file);
            }
        }
    };


    const removeAttachment = (index: number) => {
        setAttachments(prev => prev.filter((_, i) => i !== index));
    };

    const runTools = async (tools: { name: string, args: any, callId: string }[], baseHistory: Message[]) => {
        setLoading(true);
        let currentHistory = [...baseHistory];

        for (const tool of tools) {
            try {
                const result = await invoke("execute_tool", {
                    name: tool.name,
                    args: tool.args,
                    conversationId: conversationId,
                    toolCallId: tool.callId
                });

                const toolMsg: Message = {
                    role: "tool",
                    content: JSON.stringify(result),
                    tool_call_id: tool.callId
                };
                currentHistory.push(toolMsg);
                setMessages([...currentHistory]); // Update UI sequentially
            } catch (e: any) {
                console.error(e);
                const errStr = String(e);
                setErrorMsg("Tool Execution Failed: " + errStr);

                // Add error message to history
                const errorMsg: Message = {
                    role: "tool",
                    content: JSON.stringify({ error: errStr }),
                    tool_call_id: tool.callId
                };
                currentHistory.push(errorMsg);
                setMessages([...currentHistory]);
            }
        }

        // Continue conversation after ALL tools have run
        await sendMessage(currentHistory);
    };

    const sendMessage = async (currentHistory: Message[]) => {
        setLoading(true);
        setErrorMsg(null);

        try {
            console.log("Sending message with selectedModel:", selectedModel);
            let [providerId, modelId] = selectedModel.split("::");
            if (!modelId) {
                providerId = "openai";
                modelId = selectedModel;
            }

            const attachmentPayload = attachments.length > 0 ? attachments.map(a => ({
                name: a.file.name,
                media_type: a.mediaType,
                data: a.base64,
                is_binary: a.isBinary
            })) : null;

            let unlistenTransform: (() => void) | null = null;
            let tempMsgId = "streaming-" + Math.random().toString(36);

            // Setup Streaming Listener if enabled
            if (genSettings.stream) {
                // Add placeholder message
                setMessages(prev => [...prev, {
                    id: tempMsgId,
                    role: "assistant",
                    content: ""
                }]);

                const unlisten = await listen<string>("stream-chunk", (event) => {
                    console.log("📨 STREAM CHUNK:", new Date().toISOString(), event.payload.substring(0, 50));
                    // Visual indicator - flash the window title
                    document.title = `📨 Chunk (${event.payload.length} chars)`;
                    setTimeout(() => document.title = "Nebula", 100);

                    // Use flushSync to force immediate rendering in React 19
                    flushSync(() => {
                        setMessages(prev => {
                            const lastIdx = prev.length - 1;
                            const last = prev[lastIdx];
                            console.log("   → Updating message, temp ID match:", last?.id === tempMsgId);
                            if (last && last.id === tempMsgId) {
                                const newContent = (last.content || "") + event.payload;
                                console.log("   → New content length:", newContent.length);
                                return [...prev.slice(0, -1), { ...last, content: newContent }];
                            }
                            return prev;
                        });
                    });
                });
                unlistenTransform = unlisten;
            }

            // Don't await immediately - let event loop process stream events
            console.log("🚀 Starting invoke at", new Date().toISOString());
            invoke<Message>("send_message", {
                messages: currentHistory,
                providerId: providerId,
                model: modelId,
                conversationId: conversationId,
                attachments: attachmentPayload,
                temperature: genSettings.temperature,
                topP: genSettings.top_p,
                stream: genSettings.stream
            }).then(response => {
                console.log("✅ Invoke completed at", new Date().toISOString());
                if (unlistenTransform) {
                    unlistenTransform();
                }

                // Replace/Append Final Message
                setMessages(prev => {
                    if (genSettings.stream) {
                        // Replace the temp message with the final complete message
                        // (It should roughly match, but final has tool calls etc resolved cleanly)
                        return [...prev.slice(0, -1), response];
                    } else {
                        return [...prev, response];
                    }
                });

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
                    const toolsToRun = response.tool_calls.map(tc => {
                        try {
                            return {
                                name: tc.function.name,
                                args: JSON.parse(tc.function.arguments),
                                callId: tc.id || "call_" + Math.random().toString(36).substr(2, 9)
                            };
                        } catch (e) {
                            console.error("Failed to parse tool args", e);
                            return null;
                        }
                    }).filter(t => t !== null) as { name: string, args: any, callId: string }[];

                    if (toolsToRun.length > 0) {
                        // Check for Auto-Approval
                        const allAuto = toolsToRun.every(t => toolPolicies[t.name]);

                        if (allAuto) {
                            // Execute immediately
                            // We need to pass the FULL history including the assistant's tool call message
                            // The assistant message is 'response'.
                            runTools(toolsToRun, [...currentHistory, response]);
                        } else {
                            setPendingTools(toolsToRun);
                            setLoading(false); // Paused for user interaction
                        }
                    } else {
                        setLoading(false);
                    }
                } else {
                    setLoading(false);
                }
            }).catch((error: any) => {
                console.error(error);
                if (unlistenTransform) {
                    unlistenTransform();
                }
                setLoading(false);

                const errStr = String(error);
                if (errStr.includes("cancelled")) {
                    return;
                }

                // Handle Permission Denials nicely
                if (errStr.includes("denylist") || errStr.includes("allowlist")) {
                    setErrorMsg("Tool Execution Denied: " + errStr);
                } else {
                    setErrorMsg(errStr);
                }
            });
        } catch (error: any) {
            // This catch is for errors in setup, not in the invoke itself
            console.error("Error setting up message send:", error);
            setLoading(false);
            setErrorMsg(String(error));
        }
    };

    const handleStop = async () => {
        try {
            await invoke("stop_generation");
            setLoading(false);
        } catch (e) {
            console.error("Failed to stop generation", e);
        }
    };

    const handleSend = async () => {
        console.log("Triggering handleSend...");
        if (!input.trim() && attachments.length === 0) {
            console.log("Empty input and no attachments, blocking send.");
            return;
        }

        const currentAttachments = attachments.map(a => ({
            name: a.file.name,
            media_type: a.mediaType,
            data: a.base64,
            is_binary: a.isBinary
        }));

        const userMsg: Message = {
            role: "user",
            content: input,
            attachments: currentAttachments.length > 0 ? currentAttachments : undefined
        };

        const newHistory = [...messages, userMsg];
        setMessages(newHistory);
        setInput("");
        setAttachments([]); // Clear attachments after sending
        await sendMessage(newHistory);
    };



    const handleApproveAllTools = async () => {
        if (pendingTools.length === 0) return;
        const tools = [...pendingTools];
        setPendingTools([]);
        await runTools(tools, messages);
    };

    const handleDenyAllTools = () => {
        setPendingTools([]);
        setLoading(false);
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

    const handleExport = async (format: "json" | "md") => {
        console.log("Handle export clicked", format, conversationId);
        if (!conversationId) {
            setErrorMsg("No conversation ID selected");
            return;
        }
        try {
            setErrorMsg("Exporting..."); // Temporary feedback
            const content = await invoke<string>("export_conversation", { conversationId, format });
            console.log("Export content received, length:", content.length);

            const defaultName = `conversation_${conversationId}.${format}`;
            const filePath = await save({
                defaultPath: defaultName,
                filters: [{
                    name: format === 'json' ? 'JSON' : 'Markdown',
                    extensions: [format]
                }]
            });

            if (filePath) {
                await writeTextFile(filePath, content);
                setErrorMsg("Export saved to: " + filePath); // Success feedback
            } else {
                setErrorMsg("Export cancelled");
            }
            setTimeout(() => setErrorMsg(null), 3000);
        } catch (e) {
            console.error("Export failed", e);
            setErrorMsg("Export failed: " + String(e));
        }

    };



    return (
        <div
            className="flex flex-col h-full bg-[var(--color-bg-primary)] text-[var(--color-text-primary)] font-sans relative"
        >
            {isDragging && (
                <div className="absolute inset-0 z-50 bg-blue-600/20 backdrop-blur-sm border-4 border-blue-500 border-dashed m-4 rounded-xl flex items-center justify-center animate-pulse pointer-events-none">
                    <div className="bg-[var(--color-bg-secondary)]/80 p-8 rounded-2xl shadow-2xl flex flex-col items-center gap-4">
                        <Download size={48} className="text-blue-400" />
                        <h3 className="text-2xl font-bold text-[var(--color-text-primary)]">Drop files to attach</h3>
                    </div>
                </div>
            )}

            {errorMsg && (
                <div className="absolute top-4 left-1/2 transform -translate-x-1/2 z-50 bg-red-600/90 backdrop-blur text-[var(--color-text-primary)] px-4 py-3 rounded-xl shadow-2xl animate-fade-in-down flex items-center gap-3 border border-red-500/50">
                    <AlertTriangle size={20} className="text-[var(--color-text-primary)]" />
                    <span className="font-semibold">{errorMsg}</span>
                </div>
            )}

            <div className="p-4 bg-[var(--color-bg-secondary)] border-b border-[var(--color-border-primary)] flex justify-between items-center shadow-md z-10 relative">
                <div className="flex items-center gap-3">
                    <div className="w-3 h-3 rounded-full bg-green-500 animate-pulse box-shadow-lg shadow-green-500/50" />
                    <div className="w-[200px]">
                        <CustomSelect
                            value={selectedModel}
                            onChange={(val) => setSelectedModel(val)}
                            options={availableModels.map(m => ({
                                id: `${m.providerId}::${m.id}`,
                                label: m.name,
                                value: `${m.providerId}::${m.id}`,
                                icon: getProviderIcon(m.providerType, m.providerId)
                            }))}
                            placeholder={availableModels.length === 0 ? "No enabled models" : "Select Model"}
                            disabled={availableModels.length === 0}
                        />
                    </div>

                    {/* Prompt Selector */}
                    <div className="flex items-center gap-1 border-l border-[var(--color-border-secondary)] pl-3">
                        <Book size={16} className="text-[var(--color-text-secondary)]" />
                        <CustomSelect
                            value={selectedPromptId}
                            onChange={(val) => handleSetPrompt(val)}
                            options={[
                                { id: "default", label: "Default System", value: "" },
                                ...prompts.map(p => ({
                                    id: p.id,
                                    label: p.name,
                                    value: p.id
                                }))
                            ]}
                            className="w-[150px]"
                        />
                    </div>


                    <button
                        id="pin-model-btn"
                        onClick={handleSetDefaultModel}
                        className="p-2 text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors"
                        title="Set as Default Model"
                    >
                        <Pin size={16} />
                    </button>

                    {/* Settings Toggle */}
                    <div className="relative">
                        <button
                            onClick={() => setShowSettings(!showSettings)}
                            className={`p-2 rounded-lg transition-colors ${showSettings ? "bg-blue-600/20 text-blue-400" : "text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-tertiary)]"}`}
                            title="Generation Settings"
                        >
                            <Sliders size={18} />
                        </button>

                        {/* Settings Popup */}
                        {showSettings && (
                            <div className="absolute top-full right-0 mt-2 w-72 bg-[var(--color-bg-secondary)] border border-[var(--color-border-secondary)] rounded-xl shadow-2xl p-4 z-50 animate-in fade-in zoom-in-95 duration-100">
                                <h4 className="text-sm font-bold text-[var(--color-text-secondary)] mb-4 uppercase tracking-wider border-b border-[var(--color-border-primary)] pb-2">Generation Options</h4>

                                <div className="space-y-4">
                                    {/* Streaming Toggle */}
                                    <div className="flex justify-between items-center">
                                        <label className="text-sm text-[var(--color-text-secondary)]">Streaming</label>
                                        <button
                                            onClick={() => setGenSettings({ ...genSettings, stream: !genSettings.stream })}
                                            className={`w-10 h-5 rounded-full relative transition-colors ${genSettings.stream ? 'bg-blue-600' : 'bg-[var(--color-bg-tertiary)]'}`}
                                        >
                                            <div className={`w-3 h-3 bg-white rounded-full absolute top-1 transition-all ${genSettings.stream ? 'left-6' : 'left-1'}`} />
                                        </button>
                                    </div>

                                    {/* Temperature */}
                                    <div className="space-y-1">
                                        <div className="flex justify-between text-xs text-[var(--color-text-secondary)]">
                                            <span>Temperature</span>
                                            <span>{genSettings.temperature}</span>
                                        </div>
                                        <input
                                            type="range"
                                            min="0" max="2" step="0.1"
                                            value={genSettings.temperature}
                                            onChange={(e) => setGenSettings({ ...genSettings, temperature: parseFloat(e.target.value) })}
                                            className="w-full h-1 bg-[var(--color-bg-tertiary)] rounded-lg appearance-none cursor-pointer accent-blue-500"
                                        />
                                    </div>

                                    {/* Top P */}
                                    <div className="space-y-1">
                                        <div className="flex justify-between text-xs text-[var(--color-text-secondary)]">
                                            <span>Top P</span>
                                            <span>{genSettings.top_p}</span>
                                        </div>
                                        <input
                                            type="range"
                                            min="0" max="1" step="0.05"
                                            value={genSettings.top_p}
                                            onChange={(e) => setGenSettings({ ...genSettings, top_p: parseFloat(e.target.value) })}
                                            className="w-full h-1 bg-[var(--color-bg-tertiary)] rounded-lg appearance-none cursor-pointer accent-blue-500"
                                        />
                                    </div>
                                </div>
                            </div>
                        )}
                    </div>

                    {/* Export Button */}
                    <div className="relative group">
                        <button className="p-2 text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-tertiary)] rounded-lg transition-colors" title="Export Conversation">
                            <Download size={18} />
                        </button>
                        <div className="absolute right-0 top-full mt-2 w-32 bg-[var(--color-bg-secondary)] border border-[var(--color-border-secondary)] rounded-lg shadow-xl overflow-hidden invisible group-hover:visible opacity-0 group-hover:opacity-100 transition-all z-50">
                            <button
                                onClick={() => handleExport('json')}
                                className="w-full text-left px-4 py-2 text-sm text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-tertiary)] hover:text-[var(--color-text-primary)] transition-colors"
                            >
                                JSON
                            </button>
                            <button
                                onClick={() => handleExport('md')}
                                className="w-full text-left px-4 py-2 text-sm text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-tertiary)] hover:text-[var(--color-text-primary)] transition-colors"
                            >
                                Markdown
                            </button>
                        </div>
                    </div>

                    <div className="h-4 w-px bg-[var(--color-bg-tertiary)] mx-2" />

                    <button
                        onClick={() => setActiveSidePanel(activeSidePanel === 'memory' ? 'none' : 'memory')}
                        className={`p-2 rounded-lg transition-colors relative ${activeSidePanel === 'memory' ? 'bg-purple-500/20 text-purple-400' : 'text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-tertiary)]'}`}
                        title="Memory Context"
                    >
                        <Brain size={18} />
                        {recentMemories.length > 0 && (
                            <span className="absolute -top-1 -right-1 flex h-3 w-3">
                                <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-purple-400 opacity-75"></span>
                                <span className="relative inline-flex rounded-full h-3 w-3 bg-purple-500"></span>
                            </span>
                        )}
                    </button>
                </div>
            </div>

            {/* Side Panels */}
            {
                activeSidePanel === 'memory' && (
                    <MemoryPanel
                        memories={recentMemories}
                        onClose={() => setActiveSidePanel('none')}
                    />
                )
            }

            <div
                className="flex-1 overflow-y-auto p-4 space-y-6 min-h-0"
                ref={scrollRef}
            >
                {messages.map((m, i) => (
                    <ChatMessage
                        key={m.id || `msg-${i}`}
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
                pendingTools.length > 0 && (
                    <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4 z-50 animate-in fade-in duration-200">
                        <div className="bg-[var(--color-bg-secondary)] p-6 rounded-xl max-w-lg w-full border border-[var(--color-border-secondary)] shadow-2xl ring-1 ring-white/10 max-h-[80vh] flex flex-col">
                            <h3 className="text-lg font-bold mb-4 flex items-center gap-2 text-[var(--color-text-primary)] shrink-0">
                                <Terminal className="text-yellow-400" /> Use Tools? ({pendingTools.length})
                            </h3>

                            <div className="flex-1 overflow-y-auto space-y-4 mb-6 custom-scrollbar pr-2">
                                {pendingTools.map((tool, idx) => (
                                    <div key={tool.callId || idx} className="bg-black/50 p-4 rounded-lg border border-white/5">
                                        <div className="flex justify-between items-start mb-2">
                                            <p className="text-green-400 font-bold font-mono">$ {tool.name}</p>
                                            <span className="text-xs text-[var(--color-text-tertiary)] font-mono">{tool.callId?.slice(0, 8)}...</span>
                                        </div>
                                        <pre className="text-[var(--color-text-secondary)] whitespace-pre-wrap break-all text-xs font-mono bg-black/30 p-2 rounded">
                                            {JSON.stringify(tool.args, null, 2)}
                                        </pre>
                                    </div>
                                ))}
                            </div>

                            <div className="flex justify-end gap-3 shrink-0 pt-4 border-t border-[var(--color-border-primary)]">
                                <button
                                    onClick={handleDenyAllTools}
                                    className="px-4 py-2 rounded-lg bg-[var(--color-bg-tertiary)] hover:bg-[var(--color-bg-tertiary)] text-[var(--color-text-secondary)] transition-colors"
                                >
                                    Deny All
                                </button>
                                <button
                                    onClick={handleApproveAllTools}
                                    className="px-4 py-2 rounded-lg bg-blue-600 hover:bg-blue-500 text-[var(--color-text-primary)] font-semibold shadow-lg shadow-blue-500/20 transition-all hover:scale-105"
                                >
                                    Approve All
                                </button>
                            </div>
                        </div>
                    </div>
                )
            }


            <div className="p-4 bg-[var(--color-bg-secondary)] border-t border-[var(--color-border-primary)]">
                {/* Attachment Previews */}
                {attachments.length > 0 && (
                    <div className="flex gap-2 mb-2 overflow-x-auto pb-2">
                        {attachments.map((att, i) => (
                            <div key={i} className="relative group shrink-0">
                                <img src={att.preview} alt="preview" className="h-16 w-16 object-cover rounded-lg border border-[var(--color-border-secondary)]" />
                                <button
                                    onClick={() => removeAttachment(i)}
                                    className="absolute -top-1 -right-1 bg-red-500 rounded-full p-0.5 text-[var(--color-text-primary)] opacity-0 group-hover:opacity-100 transition-opacity"
                                >
                                    <X size={12} />
                                </button>
                            </div>
                        ))}
                    </div>
                )}

                <div className="max-w-4xl mx-auto flex gap-3 items-end">
                    <label className="p-3 text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer hover:bg-[var(--color-bg-tertiary)] rounded-xl transition-colors">
                        <Paperclip size={20} />
                        <input
                            type="file"
                            multiple
                            className="hidden"
                            onChange={handleFileSelect}
                        />
                    </label>

                    <textarea
                        className="flex-1 bg-[var(--color-bg-tertiary)] border border-[var(--color-border-secondary)] rounded-xl p-3 text-[var(--color-text-primary)] placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent transition-all resize-none min-h-[46px] max-h-[200px]"
                        value={input}
                        onChange={(e) => setInput(e.target.value)}
                        onKeyDown={(e) => {
                            if (e.key === "Enter" && !e.shiftKey) {
                                e.preventDefault();
                                handleSend();
                            }
                        }}
                        placeholder="Type a message..."
                        disabled={loading}
                        rows={1}
                        style={{ fieldSizing: "content" } as any} // Modern browser support for auto-sizing, fallback handled by rows/css
                    />
                    <button
                        disabled={(!input.trim() && attachments.length === 0) && !loading}
                        onClick={loading ? handleStop : handleSend}
                        className={`p-3 rounded-xl transition-all shadow-lg ${loading
                            ? "bg-red-500/10 text-red-400 hover:bg-red-500/20 border border-red-500/50 hover:shadow-red-500/20"
                            : "bg-blue-600 hover:bg-blue-500 text-[var(--color-text-primary)] shadow-blue-600/20 disabled:opacity-50 disabled:cursor-not-allowed"
                            }`}
                        title={loading ? "Stop Generating" : "Send Message"}
                    >
                        {loading ? <Square size={20} fill="currentColor" className="animate-pulse" /> : <Send size={20} />}
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

    // Local state for content to allow "Show Full" updates without mutating props
    const [displayContent, setDisplayContent] = useState(m.content);

    // Sync if prop changes (e.g. streaming update)
    useEffect(() => {
        setDisplayContent(m.content);
    }, [m.content]);

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

    // Icons based on role
    const AvatarIcon = () => {
        if (m.role === "user") return <div className="w-8 h-8 rounded-full bg-orange-500 flex items-center justify-center text-[var(--color-text-primary)]"><span className="text-xs font-bold">U</span></div>;
        if (m.role === "assistant") return <div className="w-8 h-8 rounded-full bg-blue-600 flex items-center justify-center text-[var(--color-text-primary)]"><Brain size={16} /></div>;
        if (m.role === "tool") return <div className="w-8 h-8 rounded-full bg-yellow-600 flex items-center justify-center text-[var(--color-text-primary)]"><Terminal size={14} /></div>;
        return <div className="w-8 h-8 rounded-full bg-[var(--color-bg-tertiary)] flex items-center justify-center text-[var(--color-text-primary)]"><span className="text-xs">?</span></div>;
    };

    return (
        <div className="flex gap-4 max-w-4xl mx-auto group">
            {/* Avatar Column */}
            <div className="shrink-0 pt-1">
                <AvatarIcon />
            </div>

            {/* Content Column */}
            <div className="flex-1 min-w-0">
                {/* Name & Content wrapper */}
                <div className="flex flex-col gap-1">
                    {/* Role Label - minimal, maybe optional, but good for context */}
                    {/* <div className="text-xs text-[var(--color-text-tertiary)] font-bold uppercase tracking-wider mb-1">
                        {m.role === "assistant" ? "Nebula" : "You"}
                    </div> */}

                    {/* Message Body */}
                    <div className={`${m.role === "user"
                        ? "bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)] rounded-2xl px-5 py-2.5 inline-block self-start border border-[var(--color-border-secondary)]/50"
                        : "text-[var(--color-text-primary)] pl-0"
                        }`}
                    >
                        {/* Attachments */}
                        {m.attachments && m.attachments.length > 0 && (
                            <div className="flex flex-wrap gap-2 mb-3">
                                {m.attachments.map((att: any, idx: number) => (
                                    <div key={idx} className="relative group rounded-lg overflow-hidden border border-[var(--color-border-secondary)] bg-black/30">
                                        {att.media_type.startsWith('image/') ? (
                                            <img
                                                src={att.data.startsWith('data:') ? att.data : `data:${att.media_type}; base64, ${att.data} `}
                                                alt={att.name}
                                                className="max-h-64 object-contain rounded"
                                            />
                                        ) : (
                                            <div className="p-4 flex items-center gap-3">
                                                <FileText className="text-[var(--color-text-secondary)]" />
                                                <div className="text-sm">
                                                    <p className="font-medium text-[var(--color-text-primary)]">{att.name}</p>
                                                    <p className="text-xs text-[var(--color-text-tertiary)]">{att.media_type}</p>
                                                </div>
                                            </div>
                                        )}
                                    </div>
                                ))}
                            </div>
                        )}


                        {/* Tool Output Collapse */}
                        {m.role === "tool" && !isExpanded ? (
                            <div className="flex items-center gap-3">
                                <span className="text-sm text-[var(--color-text-secondary)] italic">Tool output hidden</span>
                                <button
                                    onClick={() => setIsExpanded(true)}
                                    className="text-blue-400 hover:text-blue-300 underline text-sm"
                                >
                                    Show ({displayContent?.length || 0} chars)
                                </button>
                            </div>
                        ) : (
                            <>
                                {showRaw ? (
                                    <pre className="whitespace-pre-wrap font-mono text-sm text-[var(--color-text-secondary)] bg-black/20 p-2 rounded border border-white/10 overflow-x-auto">
                                        {displayContent}
                                    </pre>
                                ) : (
                                    displayContent && (
                                        <div className={`prose max-w-none prose-p:leading-relaxed prose-pre:bg-[var(--color-bg-tertiary)] prose-pre:rounded-lg prose-pre:border prose-pre:border-[var(--color-border-primary)]`}>
                                            <ReactMarkdown
                                                remarkPlugins={[remarkGfm]}
                                                components={{
                                                    ul: ({ node, ...props }) => <ul className="list-disc pl-6 mb-2 space-y-1" {...props} />,
                                                    ol: ({ node, ...props }) => <ol className="list-decimal pl-6 mb-2 space-y-1" {...props} />,
                                                    li: ({ node, ...props }) => <li className="pl-1" {...props} />,
                                                    h1: ({ node, ...props }) => <h1 className="text-2xl font-bold mb-3 mt-4" {...props} />,
                                                    h2: ({ node, ...props }) => <h2 className="text-xl font-bold mb-2 mt-3" {...props} />,
                                                    h3: ({ node, ...props }) => <h3 className="text-lg font-semibold mb-2 mt-3" {...props} />,
                                                    p: ({ node, ...props }) => <p className="mb-3 last:mb-0" {...props} />,
                                                    a: ({ node, ...props }) => <a className="text-blue-400 hover:underline" target="_blank" rel="noopener noreferrer" {...props} />,
                                                    code({ node, inline, className, children, ...props }: any) {
                                                        const match = /language-(\w+)/.exec(className || '')
                                                        const codeText = String(children).replace(/\n$/, '')
                                                        const isCopied = copiedCodeVal === codeText

                                                        return !inline && match ? (
                                                            <div className="relative group/code my-4 rounded-lg overflow-hidden border border-[var(--color-border-primary)]">
                                                                <div className="bg-[var(--color-bg-tertiary)]/50 px-3 py-1.5 flex justify-between items-center border-b border-[var(--color-border-primary)]">
                                                                    <span className="text-xs text-[var(--color-text-secondary)] font-mono">{match[1]}</span>
                                                                    <button
                                                                        onClick={() => handleCopyCode(codeText)}
                                                                        className="flex items-center gap-1.5 text-xs text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors"
                                                                    >
                                                                        {isCopied ? <Check size={12} className="text-green-400" /> : <Copy size={12} />}
                                                                        {isCopied ? "Copied" : "Copy"}
                                                                    </button>
                                                                </div>
                                                                <SyntaxHighlighter
                                                                    // @ts-ignore
                                                                    style={vscDarkPlus}
                                                                    language={match[1]}
                                                                    PreTag="div"
                                                                    customStyle={{ margin: 0, padding: '1rem', background: 'var(--color-bg-primary)', fontSize: '14px' }}
                                                                    {...props}
                                                                >
                                                                    {codeText}
                                                                </SyntaxHighlighter>
                                                            </div>
                                                        ) : (
                                                            <code className={`${className} bg-[var(--color-bg-tertiary)] px-1.5 py-0.5 rounded text-[0.9em] font-mono text-[var(--color-text-primary)] border border-[var(--color-border-secondary)]/50`} {...props}>
                                                                {children}
                                                            </code>
                                                        )
                                                    }
                                                }}
                                            >
                                                {displayContent}
                                            </ReactMarkdown>

                                            {/* Show Full Logic */}
                                            {m.role === "tool" && displayContent && displayContent.endsWith("... (truncated)") && (
                                                <button
                                                    onClick={async () => {
                                                        if (m.tool_call_id) {
                                                            try {
                                                                const fullResponse = await invoke<string>("get_tool_execution", { toolCallId: m.tool_call_id });
                                                                setDisplayContent(fullResponse);
                                                            } catch (e) {
                                                                console.error("Failed to fetch full tool output", e);
                                                            }
                                                        }
                                                    }}
                                                    className="mt-2 text-xs text-blue-400 hover:text-blue-300 underline block"
                                                >
                                                    Show Full Output (fetch from audit log)
                                                </button>
                                            )}
                                        </div>
                                    )
                                )}
                                {m.role === "tool" && (
                                    <button
                                        onClick={() => setIsExpanded(false)}
                                        className="text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] text-xs mt-2 underline"
                                    >
                                        Collapse Output
                                    </button>
                                )}
                            </>
                        )}
                    </div>

                    {/* Tool Call Info */}
                    {m.tool_calls && (
                        <div className="mt-2 text-xs">
                            <button
                                onClick={() => setShowToolArgs(!showToolArgs)}
                                className="bg-[var(--color-bg-secondary)]/50 p-2 rounded border border-[var(--color-border-primary)] hover:border-[var(--color-border-secondary)] transition-colors inline-flex items-center gap-2 text-yellow-500/80 hover:text-yellow-400"
                            >
                                <Terminal size={12} />
                                <span className="font-mono">Called: {m.tool_calls[0]?.function?.name}</span>
                                <span className="opacity-50 text-[10px] ml-1">{showToolArgs ? "▼" : "▶"}</span>
                            </button>

                            {showToolArgs && (
                                <div className="mt-2 pl-4 border-l-2 border-[var(--color-border-primary)]">
                                    <div className="bg-black/40 p-3 rounded-lg border border-[var(--color-border-primary)] font-mono text-[var(--color-text-secondary)] overflow-x-auto">
                                        <pre>{m.tool_calls[0]?.function?.arguments}</pre>
                                    </div>
                                </div>
                            )}
                        </div>
                    )}

                    {/* Action Bar (Below message) */}
                    <div className="flex gap-2 mt-1 opacity-0 group-hover:opacity-100 transition-opacity duration-200 select-none">
                        <button
                            onClick={() => setShowRaw(!showRaw)}
                            className={`p-1 text-[var(--color-text-tertiary)] hover:text-blue-400 transition-colors ${showRaw ? "text-blue-400" : ""}`}
                            title="Toggle Raw View"
                        >
                            <FileText size={14} />
                        </button>
                        <button onClick={() => onCopy(m.content || "")} className="p-1 text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)] transition-colors" title="Copy">
                            <Copy size={14} />
                        </button>
                        {m.role === "user" && (
                            <button onClick={() => onEdit(m.content || "")} className="p-1 text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)] transition-colors" title="Edit">
                                <Edit2 size={14} />
                            </button>
                        )}
                        {(m.role === "assistant" || m.role === "user") && (
                            <button onClick={() => onDelete(i, m.id)} className="p-1 text-[var(--color-text-tertiary)] hover:text-red-400 transition-colors" title="Delete">
                                <Trash2 size={14} />
                            </button>
                        )}
                        {m.role === "assistant" && (
                            <button onClick={() => onRegenerate(i, m.id)} className="p-1 text-[var(--color-text-tertiary)] hover:text-green-400 transition-colors" title="Regenerate">
                                <RefreshCw size={14} />
                            </button>
                        )}
                    </div>
                </div>
            </div>
        </div>
    );
}
