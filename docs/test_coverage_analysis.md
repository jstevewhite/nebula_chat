# Test Coverage Analysis

_Snapshot of the test situation in `nebula_chat` and concrete proposals for closing the most impactful gaps._

## 1. What exists today

### Rust backend (`src-tauri/`, ~10,400 lines)

| Location | Tests | Notes |
|---|---|---|
| `src/memory/sqlite_manager.rs` | ~38 unit tests | Strong: batch helpers, SQL-injection guard, tool-call integrity, compaction checkpoints, facts upsert/migration |
| `src/llm/compactor.rs` | 5 async unit tests | Covers disabled-mode, within-limit skip, missing `last_id` recovery, empty input, missing `conversation_id` |
| `src/lib.rs` | 1 placeholder test | `test_tool_call_validation_integration` — empty body, just a comment |
| `tests/csp_test.rs` | 1 test | String-matches `tauri.conf.json` for CSP fragments |
| `tests/tool_validation_test.rs` | 1 test | Re-tests `tool_call_id_exists` against a temp DB |
| `tests/sse_cancellation_test.rs` | 1 test | **Compile-only** assertion, no behaviour exercised |
| `tests/tantivy_performance_test.rs` | 2 async tests | Smoke: enqueue 20 docs, basic CRUD. Doesn't actually search anything back |

### Frontend (`src/`, ~5,600 lines)

- **No test framework installed.** `package.json` has no `test` script, no Vitest/Jest/Testing Library/Playwright dependency.
- Zero tests for 12 components, 1 context, and the IPC surface.

### Coverage tooling

- No `cargo-tarpaulin`, `cargo-llvm-cov`, or coverage reporting in CI.
- No CI workflow at all under `.github/workflows/` (none present).

## 2. The big gaps, ranked by risk

### Tier 1 — critical, high blast radius, currently untested

1. **`OpenAiProvider::sanitize_messages` (`src/llm/openai.rs:65-244`)** — 180 lines of multi-pass tool-call orphan healing/pruning. This runs on every OpenAI request and silently mutates the conversation. A regression here corrupts chats. Zero tests.

2. **`send_message` Tauri command (`src/lib.rs:425-1217`)** — ~800-line orchestrator: dedup, streaming state machine, tool-call routing, memory injection, abort handling. Zero tests, only the integration `test_tool_call_validation_integration` placeholder.

3. **`McpClient` (`src/mcp/client.rs`)** — JSON-RPC framing, stdio/SSE/Streamable-HTTP transports, request/response correlation, timeouts, drop-cancel guard. The one existing test is compile-only. The repo's own `review.md` flagged this transport layer as high-risk.

4. **`AnthropicProvider` + `OllamaProvider` message conversion (`src/llm/anthropic.rs`, `src/llm/ollama.rs`)** — tool-use translation, system-prompt extraction, content-block shaping. `review.md` already calls out `Anthropic streaming drops tool calls` as a HIGH finding (anthropic.rs:386-397). Untested.

5. **`Settings::load_migrated` + `migrate_legacy_json` (`src/mcp/config.rs:213-387`)** — legacy-format migration, env-var override precedence (`NEBULA_OPENAI_KEY`, `NEBULA_ANTHROPIC_KEY`), keychain integration. Settings corruption is user-visible and hard to recover from. Untested.

### Tier 2 — pure functions with easy, high-value tests

These are leaf utilities where one afternoon of work would meaningfully raise coverage:

6. **`llm/capabilities.rs`** — `get_model_context_window`, `supports_reasoning_effort`, `supports_thinking_mode`, `supports_extended_thinking`. Pure string matching, table-driven tests cost almost nothing and prevent silently wrong context windows.
7. **`llm/tokenizer.rs`** — `count_tokens`, `truncate`, `count_message_tokens` (attachment/tool-call accounting). All pure.
8. **`llm/context.rs::ContextManager::prune_messages`** — system-prompt-preservation, reverse-chronological budget eviction. ~40 lines, easy edge-case coverage.
9. **`llm/tool_shaping.rs::shape_tool_output`** — UTF-8-safe truncation at `MAX_PREVIEW_CHARS`. Trivial.
10. **`memory/extraction.rs::extract_and_parse_json` and `normalize_key`** — fence-stripping (` ```json `), envelope-vs-bare-array, mixed casing/whitespace normalization. Pure, regression-prone.
11. **`memory/strategist.rs::SearchPlan::validate_and_clamp`** — clamps `queries` to `MAX_QUERIES_PER_ROUND`, per-query `limit` to `MAX_TOTAL_HITS`. Pure.
12. **`openai.rs::sanitize_base_url`** and `extract_text_from_content` — already pure helpers; quick wins paired with #1.

### Tier 3 — integration / async behaviour

13. **`mcp/manager.rs`** — server lifecycle (`initialize` / `restart_server` / `remove_server`), tool-name aggregation across servers (collision handling?), `get_server_for_tool`. Needs a mock `Transport`.
14. **`memory/tantivy_index.rs`** — only smoke-tested. No tests for `search_with_options` (role filtering, age filtering), `delete_by_*`, batching flush semantics, or what happens after `clear_index`.
15. **`memory/librarian.rs`** — 25+ public methods, none directly tested (covered transitively only when sqlite tests call into it via `compactor`).
16. **`security/keychain.rs`** — wrappers around `keyring`; needs a feature-gated mock or a `#[cfg(test)]` in-memory backend so settings tests can run hermetically.

### Tier 4 — frontend (currently 0%)

17. **Bootstrap a test framework.** Vitest + React Testing Library is the natural fit for Vite/React 19. Add a `test` script and a CI job.
18. **`ChatInterface.tsx` (2023 lines, 55 hooks, 11 `invoke` calls)** — streaming state transitions (`thinking → tool_use → responding`) and partial-chunk handling are explicitly called out in `CLAUDE.md` as bug-prone. Mock `@tauri-apps/api` and drive a state-machine test.
19. **`SettingsPage.tsx` (1296 lines)** — `CLAUDE.md` says: _"the settings page has had bugs with MCP server initialization hanging"_. Worth at least a render + form-interaction smoke test with mocked invokes.
20. **`ToolsPanel.tsx`** — Allow / Deny / Always-Allow flow is a security boundary; a few component tests would lock in behaviour.

### Tier 5 — infra & quality

21. **CI** — no GitHub Actions workflow. Add at minimum `cargo test`, `cargo clippy -D warnings`, `tsc --noEmit`, and (once added) `npm test`.
22. **Coverage reporting** — `cargo-llvm-cov` on the backend, Vitest's built-in coverage on the frontend. Even just printing the % on PRs creates pressure to add tests with new code.
23. **`tests/sse_cancellation_test.rs`** — replace the compile-only stub with an actual test (e.g. spawn a fake SSE server with `wiremock` or `axum::test`, call `stop()`, assert the loop exits within a timeout).
24. **`tests/csp_test.rs`** — currently a `String::contains` check; brittle and easy to satisfy with wrong configuration. Parse the JSON and assert on the structured policy.

## 3. Recommended order of attack

A pragmatic 4-week ramp:

1. **Week 1 — Pure-function carpet bombing.** Knock out Tier 2 (#6–#12). Cheap, big surface-area gain, builds the habit. Add `cargo-llvm-cov` and publish a baseline number.
2. **Week 2 — Tool-call integrity hardening.** Write the `sanitize_messages` test suite (#1) and Anthropic/Ollama message-conversion tests (#4). These are the regressions most likely to corrupt user conversations.
3. **Week 3 — Frontend bootstrap.** Add Vitest + RTL, wire `npm test` and CI, write a focused state-machine test for `ChatInterface`'s streaming/tool-use transitions (#17, #18).
4. **Week 4 — MCP & settings.** Mockable `Transport` trait so `McpClient` / `McpManager` can be tested without spawning processes (#3, #13); settings migration golden-file tests (#5).

## 4. Quick sanity items worth fixing now

- The placeholder `test_tool_call_validation_integration` in `lib.rs:28-32` should either become a real test or be deleted.
- `tests/sse_cancellation_test.rs` advertises behaviour it does not verify — delete it or make it real.
- `tantivy_performance_test.rs` is labelled "performance" but has no assertion on timing; either add a budget or rename it to `tantivy_smoke_test.rs`.
