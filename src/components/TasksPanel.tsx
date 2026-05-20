import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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

    // Load when conversation changes.
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

    // Live updates from the backend.
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
        <div className="tasks-panel">
            <div className="tasks-panel-header">
                <span>Tasks</span>
                <button onClick={onClose} aria-label="Close tasks panel">×</button>
            </div>
            {tasks.length === 0 ? (
                <div className="tasks-empty">No tasks yet for this conversation.</div>
            ) : (
                <ul className="tasks-list">
                    {tasks.map((t) => {
                        const marker =
                            t.status === "completed" ? "✓" : t.status === "in_progress" ? "▶" : "☐";
                        const text = t.status === "in_progress" ? t.active_form : t.content;
                        const className = `task-item task-${t.status}`;
                        return (
                            <li key={t.id} className={className}>
                                <span className="task-marker" aria-hidden="true">
                                    {marker}
                                </span>
                                <span className="task-text">{text}</span>
                            </li>
                        );
                    })}
                </ul>
            )}
        </div>
    );
}
