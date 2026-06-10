import { useState, useEffect } from "react";
import ChatInterface from "./components/ChatInterface";
import SettingsPage from "./components/SettingsPage";
import ConversationList from "./components/ConversationList";
import RightRail from "./components/RightRail";
import { Eye, EyeOff, MessageSquare, Settings } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import AppIcon from "../nebula.png";

interface Conversation {
  id: string;
  title: string;
  icon?: string;
  created_at: string;
}

export default function App() {
  const [activeTab, setActiveTab] = useState<"chat" | "settings">("chat");
  const [activeConvId, setActiveConvId] = useState<string | null>(null);
  const [contextInspectionEnabled, setContextInspectionEnabled] = useState(false);
  const [recentMemories, setRecentMemories] = useState<string[]>([]);

  useEffect(() => {
    // Initial load: Get conversations or create one
    if (activeTab === "chat" && !activeConvId) {
      initializeChat();
    }
  }, [activeTab]);

  // Load the persisted context-inspection setting on mount. The toggle button
  // for this lives in the activity bar (App), but the value is also consumed
  // by the backend during generation.
  useEffect(() => {
    invoke<any>("get_settings")
      .then((s) => setContextInspectionEnabled(Boolean(s?.context_inspection_enabled)))
      .catch((e) => console.warn("Failed to load context_inspection_enabled", e));
  }, []);

  // Memory-context events from the backend feed the RightRail's Memory tab,
  // so we listen at the App level and pass the latest list down.
  useEffect(() => {
    const unlistenPromise = listen<string[]>("memory-context", (event) => {
      setRecentMemories(event.payload);
    });
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  const initializeChat = async () => {
    try {
      const list = await invoke<Conversation[]>("list_conversations");
      if (list.length > 0) {
        setActiveConvId(list[0].id);
      } else {
        handleNewChat();
      }
    } catch (e) {
      console.error(e);
    }
  };

  const handleNewChat = async () => {
    try {
      const id = await invoke<string>("create_conversation", { title: "New Chat" });
      handleSelectConversation(id);
    } catch (e) {
      console.error(e);
    }
  };

  // Slash-command friendly variant: creates a conversation with a caller-
  // supplied title and switches to it. Returns the new id (or null on error).
  const handleCreateConversation = async (title: string): Promise<string | null> => {
    try {
      const id = await invoke<string>("create_conversation", { title });
      handleSelectConversation(id);
      return id;
    } catch (e) {
      console.error("create_conversation failed", e);
      return null;
    }
  };

  // memory3 Phase 3: when leaving a conversation, ask the backend whether the
  // configured fact_extraction_policy wants a session-end pass. The backend
  // no-ops if the policy is not "session_end", so this is safe to fire on
  // every switch.
  const handleSelectConversation = (next: string | null) => {
    const previous = activeConvId;
    if (previous && previous !== next) {
      invoke("extract_session_end", { conversationId: previous }).catch((e) => {
        console.warn("session-end extraction failed", e);
      });
    }
    setActiveConvId(next);
  };

  const toggleContextInspection = async () => {
    const newValue = !contextInspectionEnabled;
    setContextInspectionEnabled(newValue);
    try {
      const current: any = await invoke("get_settings");
      await invoke("save_settings", {
        settings: { ...current, context_inspection_enabled: newValue },
      });
    } catch (e) {
      console.error("Failed to save context_inspection_enabled:", e);
      setContextInspectionEnabled(!newValue);
    }
  };

  return (
    <div className="flex h-screen bg-[var(--color-bg-primary)] text-[var(--color-text-primary)] overflow-hidden">
      {/* Activity Bar */}
      <div className="w-16 flex flex-col items-center py-6 bg-[var(--color-bg-tertiary)] border-r border-[var(--color-border-primary)] space-y-4 z-20 shrink-0">
        <img
          src={AppIcon}
          alt="Nebula"
          className="w-10 h-10 rounded-xl mb-4 shadow-lg shadow-blue-500/20 object-cover"
        />

        <button
          onClick={() => setActiveTab("chat")}
          className={`p-3 rounded-xl transition-all duration-200 ${activeTab === "chat" ? "btn-primary shadow-lg" : "text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-secondary)] hover:text-[var(--color-text-primary)]"}`}
          title="Chat"
        >
          <MessageSquare size={20} />
        </button>

        <button
          onClick={toggleContextInspection}
          className={`p-3 rounded-xl transition-all duration-200 ${contextInspectionEnabled ? "bg-amber-500/20 text-amber-400" : "text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-secondary)] hover:text-[var(--color-text-primary)]"}`}
          title={`Context Inspection: ${contextInspectionEnabled ? "ON — will show context before sending" : "OFF"}`}
        >
          {contextInspectionEnabled ? <Eye size={20} /> : <EyeOff size={20} />}
        </button>

        <button
          onClick={() => setActiveTab("settings")}
          className={`p-3 rounded-xl transition-all duration-200 ${activeTab === "settings" ? "btn-primary shadow-lg" : "text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-secondary)] hover:text-[var(--color-text-primary)]"}`}
          title="Settings"
        >
          <Settings size={20} />
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-hidden relative flex">
        {/* Keep Chat mounted so in-flight streaming + tool approval state isn't lost when switching tabs */}
        <div className={activeTab === "chat" ? "flex flex-1 overflow-hidden" : "hidden"}>
          <ConversationList
            activeId={activeConvId}
            onSelect={handleSelectConversation}
            onCreate={handleNewChat}
          />
          <div className="flex-1 flex flex-col h-full overflow-hidden">
            <ChatInterface conversationId={activeConvId} onCreateConversation={handleCreateConversation} />
          </div>
        </div>

        <div className={activeTab === "settings" ? "flex flex-1 overflow-auto justify-center" : "hidden"}>
          <SettingsPage />
        </div>

        <RightRail recentMemories={recentMemories} conversationId={activeConvId} />
      </div>
    </div>
  );
}
