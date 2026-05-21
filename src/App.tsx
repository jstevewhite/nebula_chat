import { useState, useEffect } from "react";
import ChatInterface from "./components/ChatInterface";
import SettingsPage from "./components/SettingsPage";
import ConversationList from "./components/ConversationList";
import ToolsPanel from "./components/ToolsPanel";
import { MessageSquare, Settings, Wrench } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
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
  const [showTools, setShowTools] = useState(true);

  useEffect(() => {
    // Initial load: Get conversations or create one
    if (activeTab === "chat" && !activeConvId) {
      initializeChat();
    }
  }, [activeTab]);

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
          onClick={() => setShowTools(!showTools)}
          className={`p-3 rounded-xl transition-all duration-200 ${showTools && activeTab === "chat" ? "bg-[var(--color-bg-secondary)] text-[var(--color-accent-secondary)]" : "text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-secondary)] hover:text-[var(--color-text-primary)]"}`}
          title="Tools"
        >
          <Wrench size={20} />
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
            <ChatInterface conversationId={activeConvId} />
          </div>
          {showTools && <ToolsPanel />}
        </div>

        <div className={activeTab === "settings" ? "flex flex-1 overflow-auto justify-center" : "hidden"}>
          <SettingsPage />
        </div>
      </div>
    </div>
  );
}
