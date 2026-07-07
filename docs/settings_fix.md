# MCP settings load mitigation plan

## Problem statement
The Settings page can “fail to load” (hang) when an MCP server fails to start.

In the current backend (`src-tauri/src/mcp/manager.rs`), `McpManager` stores only *running* servers in:
- `clients: RwLock<HashMap<String, Arc<McpClient>>>`

`McpManager::initialize()` and `McpManager::restart_server()` currently acquire the `clients` write lock and then `await` server startup/handshake. If a server hangs during `McpClient::new(&config.transport).await` or during the MCP `initialize` JSON-RPC request, the write lock can be held indefinitely. That blocks `list_servers()` (read lock) and therefore any Tauri command that calls it (e.g. `get_mcp_servers`), which can make the Settings UI appear to “never load” with no visible error.

## Goals
- Settings UI must always load from persisted config even if runtime MCP servers are broken.
- No backend hangs due to holding locks across `await`.
- A broken server should degrade into status (“error/unknown”) with actionable UI (edit/retry), not a blocking failure.

## Non-goals
- Redesigning the entire MCP lifecycle.
- Implementing full telemetry.

## Proposed mitigation (phased)

### Phase 1 — Eliminate lock-across-await (primary hang fix)
Backend changes in `src-tauri/src/mcp/manager.rs` (aligning with existing structure):
- Keep `clients: RwLock<HashMap<String, Arc<McpClient>>>` as the runtime registry.
- Refactor startup so **no `RwLock` guard is held across async work**.

Concrete refactor (recommended pattern for this codebase):
- Replace `start_server_internal(&mut HashMap<...>, ...)` with a function that performs startup/handshake and returns a ready client:
  - `async fn start_client(name: &str, config: &McpServerConfig) -> Result<McpClient>`
  - Inside it, do what `start_server_internal` does today:
    - `McpClient::new(&config.transport).await`
    - `client.request("initialize", ...).await`
    - `client.notify("notifications/initialized", ...).await`
- In `initialize(settings: Settings)`:
  - Determine which servers are already running (read lock briefly).
  - For each server to start: call `start_client(...).await` **with no lock held**.
  - On success: acquire write lock briefly and `clients.insert(name, Arc::new(client))`.

Fix the same issue in `restart_server(name, config)`:
- Under a short write lock, remove the existing entry (`clients.remove(&name)`).
- Start the new client outside the lock.
- Under a short write lock, insert the new client.

Prevent duplicate concurrent starts (important once startup happens outside the lock):
- Add a lightweight per-server “starting” guard in `McpManager` (e.g. `starting: RwLock<HashSet<String>>` or encode it into a status map in Phase 4).
  - Under a short write lock: if `name` is already in `starting` or in `clients`, skip.
  - Mark `starting.insert(name)`.
  - Start outside the lock.
  - Clear `starting` and insert/update status.

Acceptance criteria:
- A hung MCP server can no longer block `list_servers()`.
- `get_mcp_servers` always returns promptly (even if some servers are broken).

### Phase 2 — Bound startup latency safely (timeouts + cleanup)
Backend changes:
- Add timeouts around potentially-hanging operations:
  - Transport startup / connection (stdio spawn or SSE connect)
  - MCP `initialize` JSON-RPC request
- Ensure timeouts do not leak stdio child processes:
  - Implement the timeout where the `tokio::process::Child` handle is owned (likely inside `McpClient::new` or inside `start_client`).
  - On timeout/failure: best-effort `kill` + `wait` the child process before returning the error.

Practical defaults (can be adjusted later):
- Spawn/connect: 2–3s
- Handshake (`initialize`): 5–10s

Acceptance criteria:
- A broken server fails fast and does not leave an untracked process running.

### Phase 3 — Decouple Settings UI from runtime availability (frontend resiliency)
Frontend changes in `src/components/SettingsPage.tsx`:
- Load persisted settings first:
  - `const settings = await invoke("get_settings")`
  - Render server rows from `settings.mcp_servers` (this is the source of truth for “what servers exist”).
- Fetch runtime server list best-effort:
  - Wrap `invoke("get_mcp_servers")` in its own try/catch (or `Promise.allSettled`).
  - If it errors or times out client-side, set status to `unknown` rather than blocking render.

Acceptance criteria:
- Settings page renders even if all MCP servers are down or hung.

### Phase 4 — Add explicit server status + last_error (better UX)
Backend changes:
- Add a status map in `McpManager`, separate from `clients`:
  - Suggested: `statuses: RwLock<HashMap<String, ServerStatus>>`
  - `ServerStatus` includes state (Starting/Connected/Error/Unknown), `last_error`, and timestamps.
- Add a Tauri command (best-effort response) e.g. `get_mcp_server_statuses`:
  - It should return a payload even if some internal lookups fail.
  - Prefer representing failures in the returned per-server fields (e.g. `last_error`) instead of failing the whole command.

Frontend changes:
- Display `Connected/Error/Unknown` per server.
- Show `last_error` on hover/expand.
- Provide actions:
  - Retry (calls `edit_mcp_server` / `restart_server` flow).

Optional improvement:
- Emit Tauri events on status changes to avoid polling.

Acceptance criteria:
- Users can see why a server is failing and retry without restarting the app.

## Test plan
- Repro: configure an MCP server that hangs (e.g. command that never responds to initialize) and open Settings.
- Verify:
  - Before fix: Settings load hangs.
  - After Phase 1: Settings loads; runtime servers list returns promptly.
  - After Phase 2: status transitions to error within the timeout window and no stray process remains.
- Regression checks:
  - Working servers still connect and tools list populates.
  - Restart flow works without blocking other commands.

## Rollout notes
- Implement Phase 1 + Phase 3 first (highest impact, minimal surface area).
- Add Phase 2 timeouts + cleanup next to prevent long stalls/leaks.
- Add Phase 4 statuses for improved UX and debuggability.
