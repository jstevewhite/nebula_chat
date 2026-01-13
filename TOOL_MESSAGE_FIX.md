# Tool Message `tool_call_id` Fix

## Problem
When message summarization was disabled (set to 0), the app would send tool messages to LLM providers (Groq, Moonshot AI, etc.) without the required `tool_call_id` field, causing 400 errors:
- `"'messages.4' : for 'role:tool' the following must be satisfied[('messages.4.tool_call_id' : property 'tool_call_id' is missing)]"`
- `"Invalid request: tool_call_id  is not found"`

## Root Cause
1. Some tool messages in the database had `NULL` or empty `tool_call_id` values
2. When summarization was disabled (`context_uncompressed_msg_count = 0`), messages bypassed the compactor's validation
3. The `sanitize_messages` function in `openai.rs` was supposed to filter these out, but they were slipping through
4. When serializing to JSON, empty/missing `tool_call_id` fields weren't included, violating strict provider requirements

## Fixes Applied

### 1. Filter on Database Load (`lib.rs:356-369`)
Added validation in `get_chat_history` to skip tool messages without valid `tool_call_id`:
```rust
if role == "tool" {
    let has_valid_id = match &tool_call_id {
        Some(id) => !id.trim().is_empty(),
        None => false,
    };
    
    if !has_valid_id {
        tracing::warn!("Skipping tool message without valid tool_call_id");
        continue;
    }
}
```

### 2. Filter on Send (`lib.rs:419-435`)
Added immediate filtering in `send_message` before any processing:
```rust
messages.retain(|msg| {
    if msg.role == "tool" {
        let has_valid_id = match &msg.tool_call_id {
            Some(id) => !id.trim().is_empty(),
            None => false,
        };
        
        if !has_valid_id {
            tracing::warn!("Filtering out tool message without valid tool_call_id");
            return false;
        }
    }
    true
});
```

### 3. Enhanced Logging (`openai.rs:259-276, 492-509`)
Added detailed logging after sanitization to track:
- Message count before/after sanitization
- Tool messages and their `tool_call_id` values
- Which messages are being filtered

### 4. Database Cleanup Command (`lib.rs:2448-2483`)
Added new Tauri command `cleanup_invalid_tool_messages` to:
- Scan all conversations
- Find tool messages with missing/empty `tool_call_id`
- Delete them from the database
- Return count of deleted messages

## Testing

### To run the cleanup:
1. Copy `cleanup_tool_messages.html` to the app's resources
2. Open it in the running Tauri app
3. Click "Run Cleanup"
4. Refresh your conversations

### Or use developer console:
```javascript
await window.__TAURI__.core.invoke('cleanup_invalid_tool_messages');
```

## Prevention
- The frontend already correctly sets `tool_call_id: tool.callId` for all tool messages
- The filters ensure invalid messages never reach the LLM providers
- Both database load and send paths now validate tool messages

## Files Modified
- `src-tauri/src/lib.rs` - Added filters and cleanup command
- `src-tauri/src/llm/openai.rs` - Enhanced logging
- `cleanup_tool_messages.html` - Cleanup utility (new)
