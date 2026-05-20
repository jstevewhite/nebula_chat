use tauri_appnebula_lib::memory::sqlite_manager::SqliteManager;
use serde_json::json;

#[test]
fn test_tool_call_id_validation() {
    // Create a temporary database for testing
    let temp_db_path = "/tmp/test_tool_validation.db";
    
    // Clean up any existing test database
    let _ = std::fs::remove_file(temp_db_path);
    
    // Create SQLite manager
    let sqlite = SqliteManager::new(temp_db_path).unwrap();
    
    // Apply migrations: v2 adds tool_calls/tool_call_id, v3 adds reasoning_content
    // (required by the current save_full_message signature).
    sqlite.migrate_v2().unwrap();
    sqlite.migrate_v3().unwrap();

    // Create a test conversation using the save_message method
    let conv_id = "test_conversation_1";
    sqlite.conn.execute(
        "INSERT INTO conversations (id, title, created_at) VALUES (?1, ?2, ?3)",
        [conv_id, "Test Conversation", "2024-01-01T00:00:00Z"],
    ).unwrap();

    // Test 1: Tool call ID should not exist initially.
    // Note: this populates the tool_call_cache with a negative entry. Prior to the
    // cache-invalidation fix on save_full_message, that cached false would poison
    // subsequent lookups even after the tool_call was actually saved below.
    assert!(!sqlite.tool_call_id_exists(conv_id, "test_tool_call_1").unwrap());

    // Test 2: Add an assistant message with tool calls using save_full_message
    let tool_calls_json = json!([
        {
            "id": "test_tool_call_1",
            "type": "function",
            "function": {
                "name": "test_function",
                "arguments": "{}"
            }
        }
    ]).to_string();

    sqlite.save_full_message(
        conv_id,
        "assistant",
        Some("Some content"),
        Some(&tool_calls_json),
        None,
        None, // reasoning_content
    ).unwrap();
    
    // Test 3: Tool call ID should now exist
    assert!(sqlite.tool_call_id_exists(conv_id, "test_tool_call_1").unwrap());
    
    // Test 4: Non-existent tool call ID should still return false
    assert!(!sqlite.tool_call_id_exists(conv_id, "non_existent_tool_call").unwrap());
    
    // Test 5: Tool call ID from different conversation should not exist
    assert!(!sqlite.tool_call_id_exists("different_conversation", "test_tool_call_1").unwrap());
    
    // Clean up
    std::fs::remove_file(temp_db_path).unwrap();
}