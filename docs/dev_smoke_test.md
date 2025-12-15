# Developer Smoke Test Checklist (Phase 0 Foundations)

This checklist is intended for developers to quickly verify that the foundational pieces added in Phase 0 are working correctly.

## Steps
1. **Build & Run**
   ```bash
   cd src-tauri
   cargo run
   ```
   Verify the Tauri window opens without panics.
2. **Create a New Chat**
   - Click **New Conversation**.
   - Ensure the conversation appears in the sidebar.
3. **Send a Message**
   - Type a short prompt (e.g., `Hello`) and hit **Send**.
   - Confirm the assistant replies (streamed or full).
4. **Stream Response**
   - Enable the **Stream** toggle in the UI (if present).
   - Verify the response appears incrementally.
5. **Use a Tool**
   - Send a message that triggers a tool (e.g., `search web for "rust"`).
   - Approve the tool call if a modal appears.
   - Confirm the tool result is displayed and incorporated into the assistant reply.
6. **View Memory Panel**
   - Open the **Memory** or **Context** panel.
   - Verify that recent messages are listed and searchable.
7. **Edit / Delete Conversation**
   - Rename the conversation.
   - Delete the conversation and ensure it disappears from the UI.
8. **Check Logs**
   - Open the console output (or log file if exported).
   - Look for `info!`, `debug!`, and `error!` entries corresponding to the actions above.
9. **Keychain Verification**
   - Open the OS Keychain Access (macOS) and locate an entry named `nebula_chat_provider_key` (or similar).
   - Ensure the provider API key is stored there, not in `settings.json`.
10. **Migration Run**
    - Delete the existing SQLite DB (`nebula_chat.db` in the app data folder).
    - Restart the app; the migration framework should create the DB and run any pending migrations (check logs for `Running migrations` messages).

If all steps succeed, Phase 0 foundations are considered functional.
