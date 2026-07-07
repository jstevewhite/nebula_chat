# Import instruction skills from Claude Code

**Date:** 2026-06-18
**Status:** Approved, ready for implementation plan

## Problem

Nebula has its own skills system: single-file `<slug>.md` documents under
`~/.config/.../skills/` (plus a bundled `built-ins/` set), parsed into a
`Skill` struct and surfaced to the model via the `use_skill` / `list_skills`
tools and an "Available skills" block in the system prompt
(`SkillStore::render_for_system_prompt`).

Claude Code, installed on the same machine, has its own library of skills under
`~/.claude/skills/`. Many of these are pure **instruction** skills — process
guidance whose entire value is the prose body (e.g. `brainstorming`,
`systematic-debugging`, `receiving-code-review`, `caveman`). The user wants
Nebula to optionally surface those skills too, so a skill authored once for
Claude Code shows up in Nebula without being re-authored.

The two systems use the same idea (markdown + YAML frontmatter) but a different
on-disk shape and a different runtime contract:

| | Nebula skill | Claude Code skill |
|---|---|---|
| On disk | one flat file `<slug>.md` | a **directory** `<name>/SKILL.md` (+ optional bundled files) |
| Frontmatter | `name`, `description`, `built_in` | `name`, `description` (+ `version`, sometimes `allowed-tools`) |
| `use_skill` returns | the whole body, as text | the SKILL.md body, which may point the agent at bundled scripts/docs |
| Location | one dir + `built-ins/` | `~/.claude/skills/`, project dirs, plugin caches; many are symlinks |
| Runtime assumed | none — pure text injection | an agent with file-read + bash + subagent tools |

Claude skills split into two populations: **instruction skills** whose SKILL.md
body is self-sufficient as guidance, and **script/subagent skills** that are
launchers for bundled code or fan-out subagents Nebula cannot run. This feature
targets the first population and deliberately keeps the second out of the way.

## Goals

- Optionally discover Claude Code skills in `~/.claude/skills/` and surface them
  through Nebula's existing skill pipeline (`use_skill`, `list_skills`, the
  system-prompt block) with no changes to those consumers.
- Default the surface to the **instruction** skills via a conservative
  classification heuristic, while letting the user override any individual
  decision from a checklist in Settings.
- Be opt-in: Nebula does not read `~/.claude` until the user enables it.
- Be honest: skills that look like script/subagent launchers are visible but
  off by default and labelled.
- Backward compatible: existing `settings.json` and existing native skills load
  and behave unchanged.

## Non-goals (YAGNI)

- **No plugin-cache scanning.** Only `~/.claude/skills/` is scanned (it already
  aggregates many plugin skills via symlinks). Plugin caches
  (`~/.claude/plugins/cache/...`), project `.claude/skills/`, and
  user-configurable scan roots are out.
- **No resource/script inlining or execution.** `use_skill` returns the SKILL.md
  body only. Bundled scripts and companion files are not resolved, inlined, or
  run. Skills that depend on them work only to the extent their body is
  self-sufficient.
- **No subagent primitive.** Skills built around dispatching subagents are not
  made to work; they are simply classified off-by-default.
- **No namespacing of imported slugs.** Collisions are resolved by native-wins
  (see C1), not by prefixing.
- **Imported skills are read-only in Nebula.** They are edited in Claude Code;
  Nebula reflects them. (Many are symlinks into plugin caches anyway.)

## Decisions made during design

These were resolved with the user and are fixed inputs to the plan:

- **Population:** import instruction skills only ("borrow the good ones"), not
  script/subagent skills.
- **Curation:** heuristic sets the default on/off state; a checklist in Settings
  lets the user override per skill (heuristic + override).
- **Scope:** `~/.claude/skills/` only, first cut.
- **Read-only** imported skills, plus **C1 collision rule:** native skills (user
  or built-in) always win a slug; the colliding Claude skill is hidden but shown
  as shadowed.
- **Opt-in:** a master toggle, default off; while off Nebula never scans or
  watches `~/.claude`.

## Design

All work lives in the existing `skills` module, the settings struct, the
`SkillsSettings` UI, and `lib.rs` wiring. The integration approach is to
**extend the existing `SkillStore`** to scan a second root and tag each skill's
origin, rather than build a parallel store — this reuses `use_skill`,
`list_skills`, `render_for_system_prompt`, and the watcher unchanged.

### Data model (`skills/api.rs`)

- Add `origin: SkillOrigin` to `Skill` and `SkillSummary`, where `SkillOrigin`
  is `Native | Claude`. Serde defaults to `Native` so existing code is
  unaffected.
- Add a `ClaudeSkillEntry` type for the Settings checklist:
  `{ slug, name, description, heuristic_default: bool, effective_enabled: bool,
  shadowed_by_native: bool }`.

### Settings (`mcp/config.rs`)

Both fields added via `load_migrated`, fully backward-compatible:

- `import_claude_skills: bool` — default `false`. The M2 master toggle.
- `claude_skill_overrides: HashMap<String, bool>` — default empty. Per-skill
  override map. **Presence = explicit user choice; absence = use the heuristic
  default.** Storing only deviations means the map survives skills appearing or
  disappearing and survives heuristic changes.

### Discovery (`skills/store.rs`)

New `scan_claude_skills(dir) -> Vec<Skill>`:

- For each entry in `~/.claude/skills/`, canonicalize the path (to follow
  symlinks); if it is a directory containing `SKILL.md`, parse that file with
  the existing frontmatter logic.
- `slug` = directory name; must pass the existing `is_valid_slug`, else skip and
  `tracing::warn`.
- `name` / `description` from frontmatter (description still required), `body` =
  SKILL.md content after frontmatter, `origin = Claude`, `built_in = false`.
- Frontmatter parsing additionally captures `allowed-tools` (when present) to
  feed the heuristic; other unknown keys (`version`, …) are ignored as today.

### Classification heuristic (`skills/store.rs`)

`claude_skill_default_enabled(skill, allowed_tools) -> bool`, deliberately
conservative toward **inclusion** (most are instruction skills, and the checklist
recovers either kind of mistake). Returns **off by default** only on a strong
"this is a doer, not an instruction" signal:

- frontmatter `allowed-tools` includes an execution tool (Bash / Write / Edit /
  etc.), **or**
- the body has an explicit run-this-script imperative (e.g. `` `foo.py` `` /
  `./x.sh` / `python …` / `bash …`), **or**
- the body has subagent-dispatch language ("dispatch a subagent", "spawn … agent",
  "Task tool", "subagent").

A bare bundled `scripts/` directory alone does **not** disqualify a skill (this
is why `brainstorming` stays on). The heuristic is best-effort and documented as
such; the checklist is the source of truth for the user.

### Collisions — C1 (`skills/mod.rs`)

Native skills (user or built-in) always win a slug. A Claude skill whose slug
collides with a native one is excluded from the active cache but retained in the
discovered list flagged `shadowed_by_native: true`.

### Two cached views in `SkillStore`

1. The existing active `Vec<Skill>` — now native skills **plus enabled,
   non-shadowed Claude skills**. Feeds `use_skill`, `list_skills`, and
   `render_for_system_prompt` with no changes to those.
2. A new `claude_discovered: Vec<ClaudeSkillEntry>` — every discovered Claude
   skill with its computed state, for the Settings checklist.

`SkillStore` gains an import-config cell `{ enabled, dir, overrides }` and a
`set_claude_import(cfg)` method that updates it, arms or drops the Claude-dir
watcher, and reloads. `reload()` scans Claude skills only when `enabled`,
classifies each, applies overrides (override > heuristic), resolves C1, and fills
both views.

### Tauri commands & startup (`lib.rs`, `skills/api.rs`)

- At startup, `SkillStore::new` reads `Settings` and calls `set_claude_import`
  with `{ enabled, dir: ~/.claude/skills, overrides }` so imports are live on
  launch when the toggle is on.
- New command `list_claude_skills() -> Vec<ClaudeSkillEntry>` for the checklist
  (reads cached view #2).
- The master toggle and per-skill checkboxes flow through the **existing
  settings-save path**, which then calls `set_claude_import(...)` and re-emits
  the existing "skills changed" Tauri event the UI already listens to. No new
  event type.
- `~/.claude/skills` is resolved from the home directory (consistent across
  macOS / Linux / Windows for Claude Code).

### Frontend (`SkillsSettings.tsx`)

- A master switch: **"Import skills from Claude Code (`~/.claude/skills`)"**,
  default off.
- When on, a read-only checklist below it, one row per discovered skill:
  checkbox (effective state) · name · truncated description · a "from Claude
  Code" badge. A muted right-side note when relevant: *"shadowed by native
  skill"* (checkbox disabled) or *"looks like it needs scripts — off by
  default"* (heuristic-off; still checkable).
- Toggling a checkbox writes `claude_skill_overrides[slug] = value` and saves.
- Toggle on but the directory is missing/empty → "No Claude skills found in
  `~/.claude/skills`."
- The native skills list is unchanged; Claude rows have no edit/delete controls.

### Watcher

- When import is enabled, arm a second `watcher::start_watching` on
  `~/.claude/skills` (reuses the existing 250ms-debounced reload). It is dropped
  when the toggle is turned off.
- **Known limitation (documented):** `notify` may not fire on content edits
  *inside* a symlinked plugin skill; adding/removing a skill is detected.
  Restarting the app or toggling the switch picks up any missed content edits.
  Acceptable for v1.

## Data flow

1. App start → `SkillStore::new` reads `Settings` → `set_claude_import` →
   `reload()` populates both cached views (active cache used immediately by the
   system prompt and tools).
2. User opens Settings → Skills → frontend calls `list_claude_skills` to render
   the checklist.
3. User flips the master toggle or a checkbox → settings saved →
   `set_claude_import` → watcher armed/dropped → `reload()` → "skills changed"
   event → UI and next system-prompt assembly reflect the change.
4. A change on disk under `~/.claude/skills` (while enabled) → debounced
   watcher → `reload()` → same event.
5. Model calls `use_skill(slug)` → `get(slug)` reads the active cache → returns
   the body (native or imported alike).

## Error handling

All failures are non-fatal — a broken Claude skill never breaks chat or the
Settings panel:

- Missing or unreadable `~/.claude/skills` → empty discovered list, no error.
- Unparseable `SKILL.md` / missing description / invalid slug / symlink
  resolution failure → skip the entry and `tracing::warn`, consistent with the
  current `scan_all` behaviour.

## Testing (`cargo test`, in `store.rs` / `mod.rs`)

- **Discovery:** dir-based `SKILL.md` is picked up; a directory without
  `SKILL.md` is skipped; a symlinked skill is resolved; slug is taken from the
  directory name; a skill missing a description is skipped.
- **Heuristic:** instruction-only body → default on; `allowed-tools: [Bash]` or a
  "run `x.py`" imperative → default off; subagent-dispatch language → default
  off; a bare `scripts/` dir with a self-sufficient body → still on.
- **Collision (C1):** a native skill with the same slug shadows the Claude one —
  absent from the active cache, present and flagged in the discovered list.
- **Overrides:** an explicit override flips the heuristic default in both
  directions.
- **Settings migration:** `import_claude_skills` defaults false;
  `claude_skill_overrides` defaults empty; an older `settings.json` without
  these fields loads unchanged.

## Out of scope (future tiers)

- **Tier 2:** resolve/inline bundled resources (rewrite relative paths to
  absolute, append a file manifest), scan project roots and plugin caches,
  configurable scan paths, namespacing.
- **Tier 3:** execute bundled scripts / provide a subagent primitive —
  effectively reimplementing Claude Code's skill runtime. Not planned.
