import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ListChecks, X, Check, CircleDot, Circle } from "lucide-react";

interface PersistedTask {
    id: string;
    conversation_id: string;
    position: number;
    content: string;
    active_form: string;
    status: "pending" | "in_progress" | "completed";
    updated_at: string;
}

interface TasksUpdatedPayload {
    conversation_id: string;
    tasks: PersistedTask[];
}

interface TasksPanelProps {
    conversationId: string | null;
    onClose: () => void;
}

export default function TasksPanel({ conversationId, onClose }: TasksPanelProps) {
    const [tasks, setTasks] = useState<PersistedTask[]>([]);

    useEffect(() => {
        if (!conversationId) {
            setTasks([]);
            return;
        }
        let cancelled = false;
        invoke<PersistedTask[]>("get_conversation_tasks", { conversationId })
            .then((rows) => {
                if (!cancelled) setTasks(rows);
            })
            .catch((e) => console.error("get_conversation_tasks failed", e));
        return () => {
            cancelled = true;
        };
    }, [conversationId]);

    useEffect(() => {
        const unlistenPromise = listen<TasksUpdatedPayload>("tasks-updated", (event) => {
            if (event.payload.conversation_id === conversationId) {
                setTasks(event.payload.tasks);
            }
        });
        return () => {
            unlistenPromise.then((unlisten) => unlisten());
        };
    }, [conversationId]);

    return (
        <div className="w-80 h-full border-l border-gray-800 bg-gray-900 flex flex-col shadow-xl shrink-0 animate-in slide-in-from-right duration-200">
            <div className="p-4 border-b border-gray-800 flex justify-between items-center bg-gray-900/50 backdrop-blur">
                <h3 className="text-sm font-semibold text-gray-200 flex items-center gap-2">
                    <ListChecks size={16} className="text-blue-400" />
                    Tasks
                </h3>
                <button
                    onClick={onClose}
                    aria-label="Close tasks panel"
                    className="text-gray-500 hover:text-white transition-colors p-1 hover:bg-gray-800 rounded"
                >
                    <X size={16} />
                </button>
            </div>

            <div className="flex-1 overflow-y-auto p-4 space-y-2">
                {tasks.length === 0 ? (
                    <div className="text-center text-gray-500 mt-10 text-sm italic">
                        No tasks yet for this conversation.
                    </div>
                ) : (
                    <ul className="space-y-2">
                        {tasks.map((t) => {
                            const text = t.status === "in_progress" ? t.active_form : t.content;
                            const isDone = t.status === "completed";
                            const isActive = t.status === "in_progress";
                            const icon = isDone ? (
                                <Check size={14} className="text-green-400" />
                            ) : isActive ? (
                                <CircleDot size={14} className="text-blue-400" />
                            ) : (
                                <Circle size={14} className="text-gray-500" />
                            );
                            return (
                                <li
                                    key={t.id}
                                    className={`bg-gray-800/50 border p-3 rounded-lg text-sm shadow-sm transition-colors flex items-start gap-2 ${
                                        isActive
                                            ? "border-blue-500/40 text-gray-100"
                                            : isDone
                                              ? "border-gray-700/50 text-gray-500 line-through"
                                              : "border-gray-700/50 text-gray-300"
                                    }`}
                                >
                                    <span className="mt-0.5 shrink-0">{icon}</span>
                                    <span className={isActive ? "font-medium" : ""}>{text}</span>
                                </li>
                            );
                        })}
                    </ul>
                )}
            </div>
        </div>
    );
}
