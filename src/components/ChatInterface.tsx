import { useState, useRef, useEffect, memo, useCallback, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Send, Terminal, AlertTriangle, Copy, Edit2, Trash2, RefreshCw, Check, Pin, FileText, Book, Paperclip, X, Brain, Square, Sliders, Download, Eye, EyeOff, ChevronRight, ChevronDown, ListChecks } from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile, readFile, readTextFile } from "@tauri-apps/plugin-fs";
import ReactMarkdown from "react-markdown";
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism';
import remarkGfm from 'remark-gfm';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import 'katex/dist/katex.min.css';
import MemoryPanel from "./MemoryPanel";
import TasksPanel from "./TasksPanel";
import { getProviderIcon } from "../utils/providerIcons";
import { CustomSelect } from "./ui/CustomSelect";

// Built-in, in-process tool names. Auto-approved by default since they only
// touch local audit-visible content the user can inspect on disk. Kept in
// sync with the matching Rust constants:
//   - memory_doc_*  / memory_fact_*  (src-tauri/src/memory/docs/tools.rs)
//   - use_skill                       (src-tauri/src/skills/tools.rs)
const MEMORY_TOOL_NAMES = new Set([
    "memory_fact_remember",
    "memory_fact_recall",
    "memory_fact_forget",
    "memory_doc_remember",
    "memory_doc_fetch",
    "memory_doc_edit",
    "memory_doc_forget",
    "memory_doc_recall",
    "memory_doc_link_context",
    "use_skill",
    "list_skills",
]);

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
    reasoning_content?: string;
    attachments?: {
        name: string;
        media_type: string;
        data: string;
        is_binary: boolean;
    }[];
    created_at?: number | string;
}

interface StreamChunkEvent {
    request_id?: string | null;
    conversation_id?: string | null;
    chunk: string;
    chunk_type?: "text" | "reasoning";
}

interface StreamStatsEvent {
    request_id?: string | null;
    conversation_id?: string | null;
    tokens_per_second: number;
    total_tokens: number;
    duration_ms: number;
}

interface ChatInterfaceProps {
    conversationId: string | null;
}

interface ModelOption {
    id: string;
    name: string;
    providerId: string;
    providerType: string;
    context_window?: number;
    supports_reasoning_effort?: boolean;
    supports_thinking_mode?: boolean;
    supports_extended_thinking?: boolean;
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
    max_tokens?: number;
    presence_penalty?: number;
    frequency_penalty?: number;
    reasoning_effort?: string; // "low", "medium", "high"
}

export default function ChatInterface({ conversationId }: ChatInterfaceProps) {
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
    const [activeSidePanel, setActiveSidePanel] = useState<'none' | 'memory' | 'tasks'>('none');
    const [recentMemories, setRecentMemories] = useState<string[]>([]);
    const [reembedProgress, setReembedProgress] = useState<{ current: number; total: number } | null>(null);

    // Settings-driven toggles
    const [contextInspectionEnabled, setContextInspectionEnabled] = useState(false);
    const [memoryEnabled, setMemoryEnabled] = useState<boolean>(true);
    const [fullSettings, setFullSettings] = useState<any>(null);
    const [showContextModal, setShowContextModal] = useState(false);
    const [contextInspectionData, setContextInspectionData] = useState<any>(null);
    const [showMessageTimestamps, setShowMessageTimestamps] = useState<boolean>(false);

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

    // memory3 Phase 4: re-embed progress banner.
    useEffect(() => {
        const unlistenPromise = listen<{ phase: string; current: number; total: number }>(
            "memory:reembed-progress",
            (event) => {
                const { current, total } = event.payload;
                if (total === 0 || current >= total) {
                    setReembedProgress(null);
                } else {
                    setReembedProgress({ current, total });
                }
            },
        );
        return () => {
            unlistenPromise.then((unlisten) => unlisten());
        };
    }, []);

    // Listen for Context Inspection Events
    useEffect(() => {
        const unlistenPromise = listen<any>("context-inspection-request", (event) => {
            console.log("Context Inspection Request:", event.payload);
            setContextInspectionData(event.payload);
            setShowContextModal(true);
        });
        return () => {
            unlistenPromise.then(unlisten => unlisten());
        };
    }, []);
    const [errorMsg, setErrorMsg] = useState<string | null>(null);
    const [copiedCodeVal, setCopiedCodeVal] = useState<string | null>(null); // To show 'Check' icon momentarily
    const [messageStats, setMessageStats] = useState<Record<string, StreamStatsEvent>>({});
    const pendingStatsRef = useRef<StreamStatsEvent | null>(null);

    const scrollRef = useRef<HTMLDivElement>(null);

    const activeStreamRef = useRef<{
        requestId: string;
        tempMsgId: string;
        conversationId: string | null;
    } | null>(null);

    // Streaming accumulator for throttled updates
    const streamAccumulatorRef = useRef<{
        content: string;
        reasoning: string;
        lastUpdate: number;
        pendingFlush: ReturnType<typeof setTimeout> | null;
    }>({
        content: "",
        reasoning: "",
        lastUpdate: 0,
        pendingFlush: null
    });

    // Track whether we should keep auto-scrolling to bottom.
    // This prevents fighting the user's scrollbar drag / text selection while they're reading older messages.
    const autoScrollRef = useRef(true);
    const SCROLL_BOTTOM_THRESHOLD_PX = 64;

    const handleScroll = () => {
        const el = scrollRef.current;
        if (!el) return;
        const distanceFromBottom = el.scrollHeight - (el.scrollTop + el.clientHeight);
        autoScrollRef.current = distanceFromBottom <= SCROLL_BOTTOM_THRESHOLD_PX;
    };

    useEffect(() => {
        loadSettings();

        const unlistenPromise = Promise.all([
            listen("tools-updated", () => loadSettings()),
            listen("settings-updated", () => loadSettings())
        ]);

        return () => {
            unlistenPromise.then(unlisteners => unlisteners.forEach(u => u()));
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

    // If the window/tab was backgrounded during generation, re-sync on return.
    // Note: partial streaming isn't persisted; this only helps recover the *final* assistant message + tool calls.
    useEffect(() => {
        const maybeRecoverCompletedGeneration = async () => {
            const active = activeStreamRef.current;
            if (!active) return;
            if (!conversationId) return;
            if (active.conversationId !== conversationId) return;

            try {
                const history = await invoke<Message[]>("get_chat_history", { conversationId });
                const last = history[history.length - 1];
                if (!last || last.role !== "assistant") {
                    return; // still generating (DB won't have the assistant message yet)
                }

                // Replace UI state with DB-backed history (includes tool_calls + real IDs)
                setMessages(history);

                // Reconstruct pending tool approvals if we missed the normal response handler
                if (last.tool_calls && last.tool_calls.length > 0) {
                    const toolsToRun = last.tool_calls
                        .map(tc => {
                                try {
                                    // Normalize callId by removing "functions." prefix if present
                                    let callId = tc.id || "call_" + Math.random().toString(36).slice(2, 11);
                                    if (callId.startsWith("functions.")) {
                                        callId = callId.substring("functions.".length);
                                        // Fallback if stripping "functions." results in empty string
                                        if (callId.trim() === "") {
                                            callId = "call_" + Math.random().toString(36).slice(2, 11);
                                        }
                                    }
                                    
                                    return {
                                        name: tc.function.name,
                                        args: JSON.parse(tc.function.arguments),
                                        callId: callId
                                    };
                                } catch (e) {
                                    console.error("Failed to parse tool args (recovery)", e);
                                    return null;
                                }
                        })
                        .filter(t => t !== null) as { name: string, args: any, callId: string }[];

                    if (toolsToRun.length > 0) {
                        // Built-in update_tasks is auto-approved (no external side effects);
                        // the disable_builtin_task_tool setting hides it from the LLM entirely,
                        // so if we see it here it's enabled and safe to auto-run.
                        // The six memory_* tools are auto-approved by default — they only touch
                        // local audit-visible markdown docs. The user can disable that in settings.
                        const memoryAutoApprove = fullSettings?.memory_tools_auto_approve ?? true;
                        const isAutoApproved = (name: string) =>
                            name === "update_tasks"
                            || (memoryAutoApprove && MEMORY_TOOL_NAMES.has(name))
                            || !!toolPolicies[name];
                        const allAuto = toolsToRun.every(t => isAutoApproved(t.name));
                        if (allAuto) {
                            runTools(toolsToRun, history);
                        } else {
                            setPendingTools(toolsToRun);
                            setLoading(false);
                        }
                    }
                }

                activeStreamRef.current = null;
                setLoading(false);
            } catch (e) {
                console.error("Failed to recover chat state on focus", e);
            }
        };

        const onFocus = () => { void maybeRecoverCompletedGeneration(); };
        const onVisibilityChange = () => {
            if (document.visibilityState === "visible") {
                void maybeRecoverCompletedGeneration();
            }
        };

        window.addEventListener("focus", onFocus);
        document.addEventListener("visibilitychange", onVisibilityChange);

        return () => {
            window.removeEventListener("focus", onFocus);
            document.removeEventListener("visibilitychange", onVisibilityChange);
        };
    }, [conversationId, toolPolicies]);

    useEffect(() => {
        const el = scrollRef.current;
        if (!el) return;
        if (!autoScrollRef.current) return;

        // Let layout settle (markdown/code blocks/images) but don't animate continuously.
        const timer = setTimeout(() => {
            el.scrollTo({
                top: el.scrollHeight,
                behavior: "auto"
            });
        }, 50);

        return () => clearTimeout(timer);
    }, [messages]);

    // Token Counting
    const [tokenCount, setTokenCount] = useState<number>(0);
    const [contextLimit, setContextLimit] = useState<number | null>(null);

    const getModelContextLimit = (modelId: string): number | null => {
        const m = modelId.toLowerCase();
        if (m.includes("gpt-4o") || m.includes("gpt-4-turbo") || m.includes("gpt-4-0125") || m.includes("gpt-4-1106")) return 128000;
        if (m.includes("gpt-4")) return 8192;
        if (m.includes("gpt-3.5")) return 16385;
        if (m.includes("o1-preview") || m.includes("o1-mini")) return 128000;
        if (m.includes("o1")) return 200000;

        if (m.includes("claude-4.5")) return 200000;
        if (m.includes("claude-3-5") || m.includes("claude-3.5")) return 200000;
        if (m.includes("claude-3")) return 200000;

        if (m.includes("gemini-1.5-pro") || m.includes("gemini-pro-1.5") || m.includes("gemini-2")) return 2000000;
        if (m.includes("gemini-1.5-flash")) return 1000000;

        if (m.includes("deepseek-v3") || m.includes("deepseek-3")) return 128000;
        if (m.includes("deepseek")) return 32000;

        if (m.includes("llama-3.1") || m.includes("llama3.1")) return 128000;
        if (m.includes("llama-3") || m.includes("llama3")) return 8192;
        if (m.includes("mistral-large")) return 32000;

        return null;
    };

    useEffect(() => {
        // Debounce token counting
        const timer = setTimeout(async () => {
            if (messages.length === 0) {
                setTokenCount(0);
                return;
            }
            try {
                // We send the current history + current input (optional, but usually context is history)
                // For now just history
                const count = await invoke<number>("count_conversation_tokens", { messages });
                setTokenCount(count);
            } catch (e) {
                console.error("Failed to count tokens", e);
            }
        }, 500);
        return () => clearTimeout(timer);
    }, [messages]);

    // Get current model capabilities - memoized to ensure proper re-renders
    const currentModel = useMemo((): ModelOption | null => {
        if (!selectedModel || availableModels.length === 0) return null;
        const [pid, mid] = selectedModel.split("::");
        return availableModels.find(m => m.providerId === pid && m.id === mid) || null;
    }, [selectedModel, availableModels]);

    // Memoize reasoning capability check to ensure dropdown visibility updates
    const modelReasoningCapabilities = useMemo(() => {
        if (!currentModel) return null;
        const supportsReasoning = currentModel.supports_reasoning_effort ||
                                  currentModel.supports_thinking_mode ||
                                  currentModel.supports_extended_thinking;
        if (!supportsReasoning) return null;
        return {
            supportsReasoningEffort: currentModel.supports_reasoning_effort,
            supportsThinkingMode: currentModel.supports_thinking_mode,
            supportsExtendedThinking: currentModel.supports_extended_thinking,
        };
    }, [currentModel]);

    useEffect(() => {
        if (selectedModel && availableModels.length > 0) {
            const model = currentModel;
            if (model) {
                if (model.context_window) {
                    setContextLimit(model.context_window);
                } else {
                    // Fallback to local heuristic
                    const fallback = getModelContextLimit(model.id);
                    setContextLimit(fallback);
                }
            } else {
                setContextLimit(null);
            }
        }
    }, [selectedModel, availableModels]);

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
            setFullSettings(settings);
            setContextInspectionEnabled(settings.context_inspection_enabled || false);
            setMemoryEnabled(settings.memory_enabled ?? true);
            setShowMessageTimestamps(settings.show_message_timestamps ?? false);
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
                                    providerType: config.provider_type, // Extract provider type
                                    context_window: m.context_window,
                                    supports_reasoning_effort: m.supports_reasoning_effort,
                                    supports_thinking_mode: m.supports_thinking_mode,
                                    supports_extended_thinking: m.supports_extended_thinking,
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

    const toggleContextInspection = async () => {
        const newValue = !contextInspectionEnabled;
        setContextInspectionEnabled(newValue);

        if (fullSettings) {
            const updatedSettings = { ...fullSettings, context_inspection_enabled: newValue };
            try {
                await invoke("save_settings", { settings: updatedSettings });
                setFullSettings(updatedSettings);
            } catch (e) {
                console.error("Failed to save context inspection setting:", e);
                // Revert on error
                setContextInspectionEnabled(!newValue);
            }
        }
    };

    const toggleMemoryEnabled = async () => {
        const newValue = !memoryEnabled;
        setMemoryEnabled(newValue);

        if (fullSettings) {
            const updatedSettings = { ...fullSettings, memory_enabled: newValue };
            try {
                await invoke("save_settings", { settings: updatedSettings });
                setFullSettings(updatedSettings);
            } catch (e) {
                console.error("Failed to save memory_enabled setting:", e);
                // Revert on error
                setMemoryEnabled(!newValue);
            }
        }
    };

    const handleApproveContext = async () => {
        if (contextInspectionData) {
            try {
                await invoke("respond_to_context_inspection", {
                    requestId: contextInspectionData.request_id,
                    approved: true
                });
                setShowContextModal(false);
                setContextInspectionData(null);
            } catch (e) {
                console.error("Failed to approve context:", e);
            }
        }
    };

    const handleRejectContext = async () => {
        if (contextInspectionData) {
            try {
                await invoke("respond_to_context_inspection", {
                    requestId: contextInspectionData.request_id,
                    approved: false
                });
                setShowContextModal(false);
                setContextInspectionData(null);
                setLoading(false);
            } catch (e) {
                console.error("Failed to reject context:", e);
            }
        }
    };

    const handleSaveContext = async () => {
        if (!contextInspectionData) return;
        try {
            const defaultName = `context_${contextInspectionData.model || 'llm'}_${new Date().toISOString().slice(0, 19).replace(/[:.]/g, '-')}.json`;
            const filePath = await save({
                defaultPath: defaultName,
                filters: [{
                    name: 'JSON',
                    extensions: ['json']
                }]
            });
            if (filePath) {
                await writeTextFile(filePath, JSON.stringify(contextInspectionData, null, 2));
            }
        } catch (e) {
            console.error("Failed to save context:", e);
            setErrorMsg("Failed to save context: " + String(e));
            setTimeout(() => setErrorMsg(null), 3000);
        }
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

    const runToolsRef = useRef<any>(null);
    const sendMessageRef = useRef<any>(null);

    const stableRunTools = useCallback((...args: any[]) => runToolsRef.current?.(...args), []);
    const stableSendMessage = useCallback((...args: any[]) => sendMessageRef.current?.(...args), []);

    const runTools = async (tools: { name: string, args: any, callId: string }[], baseHistory: Message[]) => {
        setLoading(true);
        let currentHistory = [...baseHistory];

        for (const tool of tools) {
            try {
                const result = await invoke<any>("execute_tool", {
                    name: tool.name,
                    args: tool.args,
                    conversationId,
                    toolCallId: tool.callId,
                });

                currentHistory = [...currentHistory, {
                    role: "tool",
                    content: typeof result === 'string' ? result : JSON.stringify(result),
                    tool_call_id: tool.callId,
                    created_at: Math.floor(Date.now() / 1000)
                }];
                setMessages([...currentHistory]);
            } catch (e) {
                console.error("Tool execution failed", e);
                currentHistory = [...currentHistory, {
                    role: "tool",
                    content: `Error: ${e}`,
                    tool_call_id: tool.callId,
                    created_at: Math.floor(Date.now() / 1000)
                }];
                setMessages([...currentHistory]);
            }
        }

        // Continue conversation after ALL tools have run
        await stableSendMessage(currentHistory);
    };

    runToolsRef.current = runTools;

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
            const hasRandomUUID = typeof crypto !== "undefined" && "randomUUID" in crypto;
            // @ts-ignore
            const tempMsgId = hasRandomUUID ? `streaming-${crypto.randomUUID()}` : "streaming-" + Math.random().toString(36).slice(2);
            // @ts-ignore
            const requestId = hasRandomUUID ? crypto.randomUUID() : "req-" + Math.random().toString(36).slice(2);

            activeStreamRef.current = { requestId, tempMsgId, conversationId };

            // Setup Streaming Listener if enabled
            if (genSettings.stream) {
                // Add placeholder message
                setMessages(prev => [...prev, {
                    id: tempMsgId,
                    role: "assistant",
                    content: "",
                    created_at: Math.floor(Date.now() / 1000)
                }]);

                // Reset accumulator
                streamAccumulatorRef.current = {
                    content: "",
                    reasoning: "",
                    lastUpdate: Date.now(),
                    pendingFlush: null
                };

                // Throttled flush function (max 20 updates/sec = 50ms)
                const THROTTLE_MS = 50;
                const flushToUI = () => {
                    const acc = streamAccumulatorRef.current;
                    if (acc.content || acc.reasoning) {
                        setMessages(prev => {
                            const idx = prev.findIndex(m => m.id === tempMsgId);
                            if (idx === -1) return prev;
                            const next = [...prev];
                            next[idx] = {
                                ...next[idx],
                                content: acc.content,
                                reasoning_content: acc.reasoning || undefined
                            };
                            return next;
                        });
                        acc.lastUpdate = Date.now();
                    }
                };

                const unlisten = await listen<StreamChunkEvent>("stream-chunk", (event) => {
                    const payload = event.payload;

                    // Ignore unrelated streams
                    if (payload.request_id && payload.request_id !== requestId) return;
                    if (payload.conversation_id && conversationId && payload.conversation_id !== conversationId) return;

                    const acc = streamAccumulatorRef.current;

                    // Accumulate chunks
                    if (payload.chunk_type === "reasoning") {
                        acc.reasoning += payload.chunk;
                    } else {
                        acc.content += payload.chunk;
                    }

                    // Throttle UI updates
                    const timeSinceLastUpdate = Date.now() - acc.lastUpdate;
                    if (timeSinceLastUpdate >= THROTTLE_MS) {
                        flushToUI();
                        if (acc.pendingFlush) {
                            clearTimeout(acc.pendingFlush);
                            acc.pendingFlush = null;
                        }
                    } else if (!acc.pendingFlush) {
                        // Schedule a flush if one isn't pending
                        acc.pendingFlush = setTimeout(() => {
                            flushToUI();
                            acc.pendingFlush = null;
                        }, THROTTLE_MS - timeSinceLastUpdate);
                    }
                });

                pendingStatsRef.current = null;
                const unlistenStats = await listen<StreamStatsEvent>("stream-stats", (event) => {
                    const payload = event.payload;
                    if (payload.request_id && payload.request_id !== requestId) return;
                    pendingStatsRef.current = payload;
                });

                const origUnlisten = unlisten;
                unlistenTransform = () => {
                    origUnlisten();
                    unlistenStats();
                };
            }

            // Don't await immediately - let event loop process stream events
            console.log("🚀 Starting invoke at", new Date().toISOString());
            
            // Log messages being sent, especially tool messages
            const toolMessages = currentHistory.filter(m => m.role === 'tool');
            const assistantMessages = currentHistory.filter(m => m.role === 'assistant');
            
            if (toolMessages.length > 0) {
                console.log(`📋 Sending ${toolMessages.length} tool messages:`, toolMessages.map(m => ({
                    id: m.id,
                    tool_call_id: m.tool_call_id,
                    content_preview: m.content?.substring(0, 50)
                })));
                
                // Log assistant messages with tool_calls to check for matching IDs
                const assistantsWithCalls = assistantMessages.filter(m => m.tool_calls && m.tool_calls.length > 0);
                console.log(`🤖 Assistant messages with tool_calls: ${assistantsWithCalls.length}`);
                assistantsWithCalls.forEach((m, idx) => {
                    console.log(`  Assistant #${idx}:`, m.tool_calls?.map((tc: any) => tc.id));
                });
                
                // Check for orphaned tool messages
                const allToolCallIds = new Set(
                    assistantsWithCalls.flatMap(m => m.tool_calls?.map((tc: any) => tc.id) || [])
                );
                const orphaned = toolMessages.filter(m => !allToolCallIds.has(m.tool_call_id));
                if (orphaned.length > 0) {
                    console.warn(`⚠️ Found ${orphaned.length} orphaned tool messages (no matching assistant tool_call):`, 
                        orphaned.map(m => m.tool_call_id));
                }
            }
            console.log(`📨 Total messages: ${currentHistory.length}, roles:`, currentHistory.map(m => m.role));
            
            invoke<Message>("send_message", {
                messages: currentHistory,
                providerId: providerId,
                model: modelId,
                conversationId: conversationId,
                attachments: attachmentPayload,
                temperature: genSettings.temperature,
                topP: genSettings.top_p,
                stream: genSettings.stream,
                maxTokens: genSettings.max_tokens,
                presencePenalty: genSettings.presence_penalty,
                frequencyPenalty: genSettings.frequency_penalty,
                reasoningEffort: genSettings.reasoning_effort,
                requestId
            }).then(response => {
                console.log("✅ Invoke completed at", new Date().toISOString());
                // If maybeRecoverCompletedGeneration ran (on focus/visibility while we were
                // in flight), it nulls activeStreamRef and rebuilds messages from the DB
                // including any pending-tool reconstruction. Skip the rest of this handler
                // to avoid duplicating the assistant bubble or re-triggering tool flow.
                const handledByRecovery = activeStreamRef.current === null;
                activeStreamRef.current = null;

                // Ensure created_at exists so timestamps render immediately
                response.created_at = response.created_at ?? Math.floor(Date.now() / 1000);
                if (unlistenTransform) {
                    unlistenTransform();
                }

                // Clean up pending flush
                if (streamAccumulatorRef.current.pendingFlush) {
                    clearTimeout(streamAccumulatorRef.current.pendingFlush);
                    streamAccumulatorRef.current.pendingFlush = null;
                }

                if (handledByRecovery) {
                    setLoading(false);
                    return;
                }

                // Normalize tool_call IDs in response by removing "functions." prefix
                if (response.tool_calls && response.tool_calls.length > 0) {
                    response.tool_calls = response.tool_calls.map((tc: any) => {
                        if (tc.id && tc.id.startsWith("functions.")) {
                            return {
                                ...tc,
                                id: tc.id.substring("functions.".length)
                            };
                        }
                        return tc;
                    });
                }

                // Replace/Append Final Message (do not assume placeholder is last)
                setMessages(prev => {
                    if (genSettings.stream) {
                        const idx = prev.findIndex(m => m.id === tempMsgId);
                        if (idx === -1) {
                            return [...prev, response];
                        }
                        const next = [...prev];
                        next[idx] = response;
                        return next;
                    }
                    return [...prev, response];
                });

                // Store generation stats for this message
                if (pendingStatsRef.current) {
                    const statsKey = response.id || tempMsgId;
                    const stats = pendingStatsRef.current;
                    pendingStatsRef.current = null;
                    setMessageStats(prev => ({ ...prev, [statsKey]: stats }));
                }

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
                            // Normalize callId by removing "functions." prefix if present
                            let callId = tc.id || "call_" + Math.random().toString(36).slice(2, 11);
                            if (callId.startsWith("functions.")) {
                                callId = callId.substring("functions.".length);
                                // Fallback if stripping "functions." results in empty string
                                if (callId.trim() === "") {
                                    callId = "call_" + Math.random().toString(36).slice(2, 11);
                                }
                            }
                            
                            return {
                                name: tc.function.name,
                                args: JSON.parse(tc.function.arguments),
                                callId: callId
                            };
                        } catch (e) {
                            console.error("Failed to parse tool args", e);
                            return null;
                        }
                    }).filter(t => t !== null) as { name: string, args: any, callId: string }[];

                    if (toolsToRun.length > 0) {
                        // Built-in update_tasks is auto-approved (no external side effects);
                        // the disable_builtin_task_tool setting hides it from the LLM entirely,
                        // so if we see it here it's enabled and safe to auto-run.
                        // The six memory_* tools are auto-approved by default — they only touch
                        // local audit-visible markdown docs. The user can disable that in settings.
                        const memoryAutoApprove = fullSettings?.memory_tools_auto_approve ?? true;
                        const isAutoApproved = (name: string) =>
                            name === "update_tasks"
                            || (memoryAutoApprove && MEMORY_TOOL_NAMES.has(name))
                            || !!toolPolicies[name];
                        const allAuto = toolsToRun.every(t => isAutoApproved(t.name));

                        if (allAuto) {
                            // Execute immediately
                            // We need to pass the FULL history including the assistant's tool call message
                            // The assistant message is 'response'.
                            stableRunTools(toolsToRun, [...currentHistory, response]);
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
                activeStreamRef.current = null;

                if (unlistenTransform) {
                    unlistenTransform();
                }

                // Clean up pending flush
                if (streamAccumulatorRef.current.pendingFlush) {
                    clearTimeout(streamAccumulatorRef.current.pendingFlush);
                    streamAccumulatorRef.current.pendingFlush = null;
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

    sendMessageRef.current = sendMessage;

    const handleStop = useCallback(async () => {
        try {
            await invoke("stop_generation");
            activeStreamRef.current = null;
            setLoading(false);
        } catch (e) {
            console.error("Failed to stop generation", e);
        }
    }, []);

    const handleSend = useCallback(async () => {
        console.log("Triggering handleSend...");
        if (!input.trim() && attachments.length === 0) {
            console.log("Empty input and no attachments, blocking send.");
            return;
        }

        // memory3 Phase 3: `/remember <text>` is a local-only command that runs
        // KG fact extraction over the supplied text. It does not call the LLM
        // with the full conversation; it just turns durable assertions into
        // facts. Surface the result as an assistant note for the user.
        const rememberPrefix = "/remember ";
        if (input.trim().toLowerCase().startsWith(rememberPrefix)) {
            const text = input.trim().slice(rememberPrefix.length).trim();
            if (!text) {
                console.log("Empty /remember payload.");
                return;
            }
            setInput("");
            try {
                const result = await invoke<{ extracted: number; message: string }>(
                    "extract_facts_from_text",
                    { text },
                );
                const note: Message = {
                    role: "system",
                    content: `Remembered ${result.extracted} fact${result.extracted === 1 ? "" : "s"} from /remember: ${text}`,
                    created_at: Math.floor(Date.now() / 1000),
                };
                setMessages((prev) => [...prev, note]);
            } catch (e) {
                const note: Message = {
                    role: "system",
                    content: `/remember failed: ${String(e)}`,
                    created_at: Math.floor(Date.now() / 1000),
                };
                setMessages((prev) => [...prev, note]);
            }
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
            attachments: currentAttachments.length > 0 ? currentAttachments : undefined,
            created_at: Math.floor(Date.now() / 1000)
        };

        const newHistory = [...messages, userMsg];
        setMessages(newHistory);
        setInput("");
        setAttachments([]); // Clear attachments after sending
        await stableSendMessage(newHistory);
    }, [input, attachments, messages, stableSendMessage]);

    const handleApproveAllTools = async () => {
        if (pendingTools.length === 0) return;
        const tools = [...pendingTools];
        setPendingTools([]);
        await stableRunTools(tools, messages);
    };

    const handleDenyAllTools = () => {
        setPendingTools([]);
        setLoading(false);
    };

    const handleCopy = useCallback((text: string) => {
        navigator.clipboard.writeText(text);
    }, []);

    const handleSaveAsFact = useCallback(async (messageId: string) => {
        try {
            const result = await invoke<{ extracted: number; message: string }>(
                "extract_facts_for_message",
                { messageId },
            );
            const note: Message = {
                role: "system",
                content: `Saved ${result.extracted} fact${result.extracted === 1 ? "" : "s"} from message.`,
                created_at: Math.floor(Date.now() / 1000),
            };
            setMessages((prev) => [...prev, note]);
        } catch (e) {
            const note: Message = {
                role: "system",
                content: `Save-as-fact failed: ${String(e)}`,
                created_at: Math.floor(Date.now() / 1000),
            };
            setMessages((prev) => [...prev, note]);
        }
    }, []);

    const handleDelete = useCallback(async (index: number, id?: string) => {
        if (id) {
            try {
                await invoke("delete_message", { messageId: id });
            } catch (e) {
                console.error("Failed to delete message", e);
            }
        }
        setMessages(prev => {
            const newMsgs = [...prev];
            newMsgs.splice(index, 1);
            return newMsgs;
        });
    }, []);

    const handleRegenerate = useCallback(async (index: number, role: "user" | "assistant") => {
        // For both user and assistant messages, we want to:
        // 1. Find the user message to resubmit from
        // 2. Delete all messages after that user message
        // 3. Resubmit from that user message

        let userMessageIndex = index;

        // If this is an assistant message, find the user message before it
        if (role === "assistant") {
            userMessageIndex = index - 1;
            // Find the last user message before this assistant message
            while (userMessageIndex >= 0 && messages[userMessageIndex].role !== "user") {
                userMessageIndex--;
            }
        }

        if (userMessageIndex < 0) {
            console.error("No user message found to regenerate from");
            return;
        }

        // Get all message IDs after the user message to delete
        const messagesToDelete = messages.slice(userMessageIndex + 1)
            .filter(m => m.id)
            .map(m => m.id!);

        // Delete messages from backend if they have IDs
        if (messagesToDelete.length > 0) {
            try {
                await invoke("delete_messages", { messageIds: messagesToDelete });
            } catch (e) {
                console.error("Failed to delete messages", e);
            }
        }

        // Truncate the history
        const historyToReplay = messages.slice(0, userMessageIndex + 1);

        // Update UI state to show truncated history
        setMessages(historyToReplay);

        // Trigger regeneration with the truncated history
        // The backend will detect existing message IDs and not duplicate them
        await stableSendMessage(historyToReplay);
    }, [messages, stableSendMessage]);

    const handleEdit = useCallback((content: string) => {
        setInput(content);
        // Optionally focus input?
    }, []);

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
            className="flex flex-col h-full bg-[var(--color-bg-primary)] text-[var(--color-text-primary)] relative"
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
                    <div className="w-[420px] max-w-[45vw]">
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
                            filterable={true}
                            filterPlaceholder="Search models..."
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
                            className={`p-2 rounded-lg transition-colors ${showSettings ? "bg-[var(--color-bg-tertiary)] text-[var(--color-accent-primary)] ring-1 ring-[var(--color-accent-primary)]" : "text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-tertiary)]"}`}
                            title="Generation Settings"
                        >
                            <Sliders size={18} />
                        </button>

                        {/* Settings Popup */}
                        {showSettings && (
                            <div className="absolute top-full right-0 mt-2 w-72 bg-[var(--color-bg-secondary)] border border-[var(--color-border-secondary)] rounded-xl shadow-2xl p-4 z-50 animate-in fade-in zoom-in-95 duration-100">
                                <h4 className="text-sm font-bold text-[var(--color-text-secondary)] mb-4 uppercase tracking-wider border-b border-[var(--color-border-primary)] pb-2">Generation Options</h4>

                                {contextLimit && (
                                    <div className="mb-4 p-2 bg-[var(--color-bg-tertiary)] rounded-lg border border-[var(--color-border-primary)]">
                                        <div className="flex justify-between items-center mb-1">
                                            <span className="text-xs font-bold text-[var(--color-text-secondary)]">Context Usage</span>
                                            <span className="text-xs text-[var(--color-text-primary)]">{tokenCount.toLocaleString()} / {contextLimit.toLocaleString()}</span>
                                        </div>
                                        <div className="w-full bg-[var(--color-bg-secondary)] rounded-full h-1.5 overflow-hidden">
                                            <div
                                                className={`h-full rounded-full transition-all duration-500 ${(tokenCount / contextLimit) > 0.9 ? 'bg-red-500' :
                                                    (tokenCount / contextLimit) > 0.7 ? 'bg-yellow-500' : 'bg-green-500'
                                                    }`}
                                                style={{ width: `${Math.min((tokenCount / contextLimit) * 100, 100)}%` }}
                                            />
                                        </div>
                                    </div>
                                )}

                                <div className="space-y-4">
                                    {/* Streaming Toggle */}
                                    <div className="flex justify-between items-center">
                                        <label className="text-sm text-[var(--color-text-secondary)]">Streaming</label>
                                        <button
                                            onClick={() => setGenSettings({ ...genSettings, stream: !genSettings.stream })}
                                            className={`w-10 h-5 rounded-full relative transition-colors ${genSettings.stream ? 'bg-[var(--color-accent-primary)]' : 'bg-[var(--color-bg-tertiary)]'}`}
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

                                    {/* Max Tokens */}
                                    <div className="space-y-1">
                                        <div className="flex justify-between text-xs text-[var(--color-text-secondary)]">
                                            <span>Max Tokens</span>
                                            <span>{genSettings.max_tokens || 'Auto'}</span>
                                        </div>
                                        <input
                                            type="number"
                                            placeholder="Auto"
                                            min="1" max="32000" step="100"
                                            value={genSettings.max_tokens || ''}
                                            onChange={(e) => setGenSettings({ ...genSettings, max_tokens: e.target.value ? parseInt(e.target.value) : undefined })}
                                            className="w-full px-2 py-1 bg-[var(--color-bg-tertiary)] border border-[var(--color-border-primary)] rounded text-xs text-[var(--color-text-primary)]"
                                        />
                                    </div>

                                    {/* Presence Penalty */}
                                    <div className="space-y-1">
                                        <div className="flex justify-between text-xs text-[var(--color-text-secondary)]">
                                            <span>Presence Penalty</span>
                                            <span>{genSettings.presence_penalty ?? 0}</span>
                                        </div>
                                        <input
                                            type="range"
                                            min="-2" max="2" step="0.1"
                                            value={genSettings.presence_penalty ?? 0}
                                            onChange={(e) => setGenSettings({ ...genSettings, presence_penalty: parseFloat(e.target.value) })}
                                            className="w-full h-1 bg-[var(--color-bg-tertiary)] rounded-lg appearance-none cursor-pointer accent-blue-500"
                                        />
                                    </div>

                                    {/* Frequency Penalty */}
                                    <div className="space-y-1">
                                        <div className="flex justify-between text-xs text-[var(--color-text-secondary)]">
                                            <span>Frequency Penalty</span>
                                            <span>{genSettings.frequency_penalty ?? 0}</span>
                                        </div>
                                        <input
                                            type="range"
                                            min="-2" max="2" step="0.1"
                                            value={genSettings.frequency_penalty ?? 0}
                                            onChange={(e) => setGenSettings({ ...genSettings, frequency_penalty: parseFloat(e.target.value) })}
                                            className="w-full h-1 bg-[var(--color-bg-tertiary)] rounded-lg appearance-none cursor-pointer accent-blue-500"
                                        />
                                    </div>

                                    {/* Reasoning Controls - Show based on model capabilities */}
                                    {modelReasoningCapabilities && (
                                        <div className="space-y-1">
                                            <div className="flex justify-between text-xs text-[var(--color-text-secondary)]">
                                                <span>
                                                    {modelReasoningCapabilities.supportsReasoningEffort && "Reasoning Effort"}
                                                    {modelReasoningCapabilities.supportsThinkingMode && !modelReasoningCapabilities.supportsReasoningEffort && "Thinking Mode"}
                                                    {modelReasoningCapabilities.supportsExtendedThinking && !modelReasoningCapabilities.supportsReasoningEffort && !modelReasoningCapabilities.supportsThinkingMode && "Extended Thinking"}
                                                </span>
                                                <span className="capitalize">{genSettings.reasoning_effort || 'None'}</span>
                                            </div>
                                            <CustomSelect
                                                value={genSettings.reasoning_effort || ''}
                                                onChange={(val) => setGenSettings({ ...genSettings, reasoning_effort: val || undefined })}
                                                options={[
                                                    { id: "none", label: "None", value: "" },
                                                    { id: "low", label: "Low", value: "low" },
                                                    { id: "medium", label: "Medium", value: "medium" },
                                                    { id: "high", label: "High", value: "high" },
                                                ]}
                                                className="w-full"
                                                placeholder="None"
                                            />
                                        </div>
                                    )}
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

                    <button
                        type="button"
                        onClick={() => setActiveSidePanel(activeSidePanel === 'tasks' ? 'none' : 'tasks')}
                        className={`p-2 rounded-lg transition-colors ${activeSidePanel === 'tasks' ? 'bg-blue-500/20 text-blue-400' : 'text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-tertiary)]'}`}
                        title={activeSidePanel === 'tasks' ? "Hide tasks" : "Show tasks"}
                        aria-pressed={activeSidePanel === 'tasks'}
                    >
                        <ListChecks size={18} />
                    </button>

                    <button
                        onClick={toggleMemoryEnabled}
                        className={`p-2 rounded-lg transition-colors ${memoryEnabled ? 'bg-emerald-500/20 text-emerald-400' : 'text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-tertiary)]'}`}
                        title={`Memory: ${memoryEnabled ? 'ON - Use long-term memory and facts' : 'OFF - No long-term memory or fact extraction'}`}
                    >
                        <Brain size={18} />
                    </button>

                    <button
                        onClick={toggleContextInspection}
                        className={`p-2 rounded-lg transition-colors ${contextInspectionEnabled ? 'bg-amber-500/20 text-amber-400' : 'text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-tertiary)]'}`}
                        title={`Context Inspection: ${contextInspectionEnabled ? 'ON - Will show context before sending' : 'OFF'}`}
                    >
                        {contextInspectionEnabled ? <Eye size={18} /> : <EyeOff size={18} />}
                    </button>
                </div>
            </div>

            {/* Chat area + side panels share horizontal space — panels shrink the chat */}
            <div className="flex-1 flex flex-row min-h-0">
            <div
                className="flex-1 overflow-y-auto p-4 space-y-6 min-h-0 min-w-0 font-chat"
                ref={scrollRef}
                onScroll={handleScroll}
            >
                {reembedProgress && (
                    <div className="sticky top-0 z-10 -mt-4 -mx-4 mb-2 px-4 py-2 bg-purple-900/40 border-b border-purple-700/60 text-xs text-purple-100 flex items-center gap-2">
                        <Brain size={14} className="text-purple-300 animate-pulse" />
                        Re-embedding memory documents: {reembedProgress.current} / {reembedProgress.total}
                    </div>
                )}
                {messages.map((m, i) => (
                    <ChatMessage
                        key={m.id || `${m.role}-${m.created_at ?? "?"}-${i}`}
                        message={m}
                        index={i}
                        onCopy={handleCopy}
                        onEdit={handleEdit}
                        onDelete={handleDelete}
                        onRegenerate={handleRegenerate}
                        onSaveAsFact={handleSaveAsFact}
                        showTimestamp={showMessageTimestamps}
                        genStats={m.id ? messageStats[m.id] : undefined}
                    />
                ))}
            </div>
            {activeSidePanel === 'memory' && (
                <MemoryPanel
                    memories={recentMemories}
                    onClose={() => setActiveSidePanel('none')}
                />
            )}
            {activeSidePanel === 'tasks' && (
                <TasksPanel
                    conversationId={conversationId}
                    onClose={() => setActiveSidePanel('none')}
                />
            )}
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
                                    className="px-4 py-2 rounded-lg btn-primary font-semibold shadow-lg transition-all hover:scale-105"
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
                        className="flex-1 bg-[var(--color-bg-tertiary)] border border-[var(--color-border-secondary)] rounded-xl p-3 text-[var(--color-text-primary)] placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent transition-all resize-none min-h-[46px] max-h-[200px] font-chat"
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
                            : "btn-primary disabled:opacity-50 disabled:cursor-not-allowed"
                            }`}
                        title={loading ? "Stop Generating" : "Send Message"}
                    >
                        {loading ? <Square size={20} fill="currentColor" className="animate-pulse" /> : <Send size={20} />}
                    </button>
                </div>
            </div>

            {/* Context Inspection Modal */}
            {showContextModal && contextInspectionData && contextInspectionData.messages && (
                <div className="absolute inset-0 z-[100] flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
                    <div className="bg-[var(--color-bg-secondary)] border border-[var(--color-border-secondary)] rounded-2xl shadow-2xl w-full max-w-5xl max-h-[90vh] flex flex-col">
                        {/* Header */}
                        <div className="p-6 border-b border-[var(--color-border-primary)]">
                            <div className="flex items-center justify-between">
                                <div>
                                    <h2 className="text-2xl font-bold text-[var(--color-text-primary)] flex items-center gap-3">
                                        <Eye className="text-amber-400" size={24} />
                                        Context Inspection
                                    </h2>
                                    <p className="text-sm text-[var(--color-text-secondary)] mt-1">
                                        Review the exact context being sent to <strong>{contextInspectionData.provider}::{contextInspectionData.model}</strong>
                                    </p>
                                </div>
                                <button
                                    onClick={handleRejectContext}
                                    className="p-2 text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-tertiary)] rounded-lg transition-colors"
                                    title="Cancel"
                                >
                                    <X size={20} />
                                </button>
                            </div>
                            <div className="mt-4 flex gap-4 text-sm">
                                <div className="bg-[var(--color-bg-tertiary)] px-3 py-1.5 rounded-lg">
                                    <span className="text-[var(--color-text-secondary)]">Messages:</span>
                                    <span className="ml-2 font-bold text-[var(--color-text-primary)]">{contextInspectionData.messages?.length || 0}</span>
                                </div>
                                <div className="bg-[var(--color-bg-tertiary)] px-3 py-1.5 rounded-lg">
                                    <span className="text-[var(--color-text-secondary)]">Tools:</span>
                                    <span className="ml-2 font-bold text-[var(--color-text-primary)]">{contextInspectionData.tools_count || 0}</span>
                                </div>
                            </div>
                        </div>

                        {/* Message List */}
                        <div className="flex-1 overflow-y-auto p-6 space-y-4">
                            {contextInspectionData.messages?.map((msg: any, idx: number) => (
                                <div key={idx} className="bg-[var(--color-bg-tertiary)] border border-[var(--color-border-primary)] rounded-xl p-4">
                                    <div className="flex items-center gap-2 mb-2">
                                        <span className={`px-2 py-1 rounded text-xs font-bold uppercase ${msg.role === 'system' ? 'bg-purple-600/20 text-purple-400' :
                                            msg.role === 'user' ? 'bg-orange-600/20 text-orange-400' :
                                                msg.role === 'assistant' ? 'bg-blue-600/20 text-blue-400' :
                                                    'bg-yellow-600/20 text-yellow-400'
                                            }`}>
                                            {msg.role}
                                        </span>
                                        {msg.tool_call_id && (
                                            <span className="px-2 py-1 rounded text-xs bg-gray-600/20 text-gray-400">
                                                tool_call_id: {msg.tool_call_id}
                                            </span>
                                        )}
                                    </div>
                                    {msg.content && (
                                        <pre className="text-sm text-[var(--color-text-primary)] whitespace-pre-wrap font-mono overflow-x-auto">
                                            {msg.content}
                                        </pre>
                                    )}
                                    {msg.tool_calls && (
                                        <div className="mt-2 text-xs text-yellow-400 font-mono">
                                            Tool Calls: {msg.tool_calls}
                                        </div>
                                    )}
                                </div>
                            ))}
                        </div>

                        {/* Footer Actions */}
                        <div className="p-6 border-t border-[var(--color-border-primary)] flex justify-between items-center gap-3">
                            <button
                                onClick={handleSaveContext}
                                className="px-4 py-2.5 rounded-lg bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)] hover:bg-blue-600/20 hover:text-blue-400 border border-[var(--color-border-primary)] hover:border-blue-500/50 transition-colors font-semibold flex items-center gap-2"
                            >
                                <Download size={16} />
                                Save Raw
                            </button>
                            <div className="flex gap-3">
                                <button
                                    onClick={handleRejectContext}
                                    className="px-6 py-2.5 rounded-lg bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)] hover:bg-red-600/20 hover:text-red-400 border border-[var(--color-border-primary)] hover:border-red-500/50 transition-colors font-semibold"
                                >
                                    Cancel
                                </button>
                                <button
                                    onClick={handleApproveContext}
                                    className="px-6 py-2.5 rounded-lg bg-green-600 text-white hover:bg-green-700 transition-colors font-semibold shadow-lg"
                                >
                                    Send to Model
                                </button>
                            </div>
                        </div>
                    </div>
                </div>
            )}
        </div >
    );
}






const ThinkingBlock = memo(({ content }: { content: string }) => {
    const [collapsed, setCollapsed] = useState(true);

    return (
        <div className="mb-2 border border-[var(--color-border-secondary)] rounded-lg overflow-hidden bg-[var(--color-bg-tertiary)]/30">
            <button
                onClick={() => setCollapsed(!collapsed)}
                className="w-full flex items-center gap-2 px-3 py-2 text-xs font-bold text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-tertiary)] transition-colors select-none"
            >
                {collapsed ? <ChevronRight size={14} /> : <ChevronDown size={14} />}
                <span className="uppercase tracking-wider">Reasoning Process</span>
            </button>
            {!collapsed && (
                <div className="p-3 text-sm text-[var(--color-text-secondary)] font-mono whitespace-pre-wrap bg-black/10 border-t border-[var(--color-border-secondary)]/50">
                    {content}
                </div>
            )}
        </div>
    );
});

const ChatMessage = memo(({ message: m, index: i, onCopy, onEdit, onDelete, onRegenerate, onSaveAsFact, showTimestamp, genStats }: { message: Message, index: number, onCopy: any, onEdit: any, onDelete: any, onRegenerate: any, onSaveAsFact?: (id: string) => void, showTimestamp: boolean, genStats?: StreamStatsEvent }) => {
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

    const formatTimestamp = (value?: number | string) => {
        if (value === undefined || value === null) return "";
        const date = typeof value === "number" ? new Date(value * 1000) : new Date(value);
        if (isNaN(date.getTime())) return "";
        const pad = (n: number) => n.toString().padStart(2, "0");
        return `${date.getUTCFullYear()}:${pad(date.getUTCMonth() + 1)}:${pad(date.getUTCDate())}:${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())}:${pad(date.getUTCSeconds())}`;
    };

    const timestampText = showTimestamp ? formatTimestamp(m.created_at) : "";

    // Parsing logic for <think>, <thinking>, or <reasoning> tags.
    // The backend (OpenAI/Ollama providers) now strips <think>...</think> from
    // streamed content and routes it to reasoning_content. This regex remains
    // as a safety net for older saved messages and other tag variants.
    const { cleanContent, thinkingContent } = (() => {
        // Priority 1: Dedicated reasoning content (e.g. from DeepSeek/Qwen via backend)
        if (m.reasoning_content) {
            return { cleanContent: displayContent || "", thinkingContent: m.reasoning_content };
        }

        // Priority 2: Tag parsing (fallback)
        if (!displayContent) return { cleanContent: "", thinkingContent: null };
        const match = /<(think|thinking|reasoning)>([\s\S]*?)(<\/\1>|$)/i.exec(displayContent);
        if (match) {
            const fullMatch = match[0];
            const innerContent = match[2];
            // Remove the thinking block from the display content
            const clean = displayContent.replace(fullMatch, "").trim();
            return { cleanContent: clean, thinkingContent: innerContent };
        }
        return { cleanContent: displayContent, thinkingContent: null };
    })();

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

                    {/* Message Body */}
                    <div className={`${m.role === "user"
                        ? "bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)] rounded-2xl px-5 py-2.5 inline-block self-start border border-[var(--color-border-secondary)]/50"
                        : "text-[var(--color-text-primary)] pl-0"
                        }`}
                    >
                        {/* Thinking Block */}
                        {m.role === "assistant" && thinkingContent && (
                            <ThinkingBlock content={thinkingContent} />
                        )}

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
                                    (cleanContent || (m.role === "tool" && displayContent)) && (
                                        <div className={`prose font-chat max-w-none prose-p:leading-relaxed prose-pre:bg-[var(--color-bg-tertiary)] prose-pre:rounded-lg prose-pre:border prose-pre:border-[var(--color-border-primary)]`}>
                                            <ReactMarkdown
                                                remarkPlugins={[remarkGfm, remarkMath]}
                                                rehypePlugins={[rehypeKatex]}
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
                                                                    customStyle={{ margin: 0, padding: '1rem', background: 'var(--color-bg-primary)', fontSize: '14px', whiteSpace: 'pre-wrap', wordWrap: 'break-word' }}
                                                                    wrapLines={true}
                                                                    wrapLongLines={true}
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
                                                {m.role === "tool" ? displayContent : cleanContent}
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
                    {(() => {
                        const toolCalls = m.tool_calls ?? [];
                        if (toolCalls.length === 0) return null;
                        return (
                        <div className="mt-2 text-xs space-y-2">
                            {toolCalls.map((tc: any, tcIdx: number) => (
                                <div key={tc.id || tcIdx}>
                                    <button
                                        onClick={() => setShowToolArgs(!showToolArgs)}
                                        className="bg-[var(--color-bg-secondary)]/50 p-2 rounded border border-[var(--color-border-primary)] hover:border-[var(--color-border-secondary)] transition-colors inline-flex items-center gap-2 text-yellow-500/80 hover:text-yellow-400"
                                    >
                                        <Terminal size={12} />
                                        <span className="font-mono">Called: {tc.function?.name}</span>
                                        {toolCalls.length > 1 && (
                                            <span className="opacity-50 text-[10px]">({tcIdx + 1}/{toolCalls.length})</span>
                                        )}
                                        <span className="opacity-50 text-[10px] ml-1">{showToolArgs ? "▼" : "▶"}</span>
                                    </button>

                                    {showToolArgs && (
                                        <div className="mt-2 pl-4 border-l-2 border-[var(--color-border-primary)]">
                                            <div className="bg-black/40 p-3 rounded-lg border border-[var(--color-border-primary)] font-mono text-[var(--color-text-secondary)] overflow-x-auto">
                                                <pre>{tc.function?.arguments}</pre>
                                            </div>
                                        </div>
                                    )}
                                </div>
                            ))}
                        </div>
                        );
                    })()}

                    {/* Action Bar (Below message) */}
                    <div className="flex gap-2 mt-1 select-none items-center">
                        {showTimestamp && timestampText && (
                            <span className="text-[var(--color-text-tertiary)] text-[11px] font-mono mr-1">
                                {timestampText}
                            </span>
                        )}
                        {m.role === "assistant" && genStats && (
                            <span
                                className="text-[var(--color-text-tertiary)] text-[11px] font-mono mr-1"
                                title={`${genStats.total_tokens} tokens in ${(genStats.duration_ms / 1000).toFixed(1)}s`}
                            >
                                {genStats.tokens_per_second.toFixed(1)} tok/s
                            </span>
                        )}
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
                        {(m.role === "assistant" || m.role === "user" || m.role === "tool") && (
                            <button onClick={() => onDelete(i, m.id)} className="p-1 text-[var(--color-text-tertiary)] hover:text-red-400 transition-colors" title="Delete">
                                <Trash2 size={14} />
                            </button>
                        )}
                        {(m.role === "assistant" || m.role === "user" || m.role === "tool") && (
                            <button onClick={() => onRegenerate(i, m.role)} className="p-1 text-[var(--color-text-tertiary)] hover:text-green-400 transition-colors" title="Regenerate">
                                <RefreshCw size={14} />
                            </button>
                        )}
                        {m.role === "assistant" && m.id && onSaveAsFact && (
                            <button
                                onClick={() => onSaveAsFact(m.id!)}
                                className="p-1 text-[var(--color-text-tertiary)] hover:text-purple-400 transition-colors"
                                title="Extract long-term facts from this message"
                            >
                                <Brain size={14} />
                            </button>
                        )}
                    </div>
                </div>
            </div>
        </div>
    );
});
