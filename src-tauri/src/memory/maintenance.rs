// src/memory/maintenance.rs
use crate::AppState;
use tauri::State;
use tracing::info;

#[tauri::command]
pub async fn rebuild_memory_index(state: State<'_, AppState>) -> Result<(), String> {
    info!("Starting memory index rebuild...");
    let lib = state.librarian.lock().await;

    // 1. Clear existing index
    lib.clear_search_index().map_err(|e| e.to_string())?;

    // 2. Fetch all conversations
    let conversations = lib.list_conversations().map_err(|e| e.to_string())?;

    let mut total_messages = 0;

    for (conv_id, _, _) in conversations {
        // 3. Fetch all messages for conversation
        // We need `created_at` which `get_complete_history` doesn't return yet.
        // We'll use `get_oldest_messages` with a large limit as a workaround,
        // OR better: update `get_complete_history` to return `created_at`.
        // Let's use `get_oldest_messages` for now since it returns (id, role, content, created_at).
        // But `get_oldest_messages` doesn't return tool_calls.
        // Phase 2 requirement is "rebuild index". Search only indexes `content`.
        // So `get_oldest_messages` is sufficient for search indexing!

        let messages = lib
            .get_oldest_messages(&conv_id, 10000)
            .map_err(|e| e.to_string())?;

        for (msg_id, role, content, created_at) in messages {
            lib.index_existing_message(&conv_id, &role, &content, &msg_id, &created_at)
                .map_err(|e| e.to_string())?;
            total_messages += 1;
        }
    }

    info!("Rebuild complete. Processed {} messages.", total_messages);
    Ok(())
}
