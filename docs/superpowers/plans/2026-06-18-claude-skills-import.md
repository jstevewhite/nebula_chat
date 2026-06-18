# Claude Code Skills Import Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Optionally surface Claude Code instruction skills from `~/.claude/skills/` through Nebula's existing skill pipeline (`use_skill` / `list_skills` / system-prompt block), curated by a heuristic plus a Settings checklist.

**Architecture:** Extend the existing `SkillStore` to scan a second root (`~/.claude/skills/`), tag each `Skill` with an `origin`, classify imported skills with a conservative heuristic, and merge enabled non-shadowed ones into the same in-memory cache the LLM tools already read. A master toggle and per-skill overrides live in `Settings`; `lib.rs` translates `Settings` into a plain `ClaudeImportConfig` and pushes it into `SkillStore`. The skills module stays settings-agnostic.

**Tech Stack:** Rust (Tauri v2.11, Tokio, serde / serde_yaml, `notify` watcher, `anyhow`), React 19 + TypeScript (Vite), Tauri IPC.

## Global Constraints

- **Scope:** scan `~/.claude/skills/` only. No plugin caches, no project roots, no configurable scan paths.
- **Opt-in:** master toggle `import_claude_skills` defaults `false`. While off, never scan or watch `~/.claude`.
- **Imported skills are read-only** in Nebula (no edit/delete/clone in the UI).
- **Collision rule (C1):** a native skill (user or built-in) always wins a slug; a colliding Claude skill is excluded from the active cache and flagged `shadowed_by_native`.
- **Override store:** `claude_skill_overrides: HashMap<String, bool>` stores only deviations from the heuristic — presence = explicit user choice, absence = heuristic default.
- **Backward compatible:** existing `settings.json` and native skills load and behave unchanged. New serde fields use `#[serde(default)]`.
- **No new crate dependencies.** Home dir is resolved via Tauri's `app.path().home_dir()`.
- **Non-fatal errors:** a malformed/unreadable Claude skill is skipped + `tracing::warn`, never an error that breaks chat or Settings.
- Commit message trailer (every commit):
  ```
  🤖 Generated with [Claude Code](https://claude.com/claude-code)

  Co-Authored-By: Claude <noreply@anthropic.com>
  ```
- Spec: `docs/superpowers/specs/2026-06-18-claude-skills-import-design.md`.

---

### Task 1: Settings fields for the toggle + overrides

**Files:**
- Modify: `src-tauri/src/mcp/config.rs` (add two fields to `struct Settings` after `image_proxy_allowlist`, around line 242; add a test to the `mod tests` block at line 675)

**Interfaces:**
- Consumes: nothing.
- Produces: `Settings.import_claude_skills: bool` (default false) and `Settings.claude_skill_overrides: std::collections::HashMap<String, bool>` (default empty), read by Tasks 5/6.

`HashMap` and `default_false_bool` are already imported/defined in this file.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/mcp/config.rs`:

```rust
    #[test]
    fn settings_default_claude_import_fields() {
        // An older settings.json with none of the new fields must load with
        // import disabled and no overrides.
        let json = r#"{ "providers": {}, "mcp_servers": {} }"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!(!s.import_claude_skills);
        assert!(s.claude_skill_overrides.is_empty());
    }

    #[test]
    fn settings_roundtrip_claude_import_fields() {
        let mut s = Settings::default();
        s.import_claude_skills = true;
        s.claude_skill_overrides.insert("brainstorming".into(), false);
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert!(back.import_claude_skills);
        assert_eq!(back.claude_skill_overrides.get("brainstorming"), Some(&false));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib mcp::config::tests::settings_default_claude_import_fields mcp::config::tests::settings_roundtrip_claude_import_fields`
Expected: FAIL to compile — `no field 'import_claude_skills' on type 'Settings'`.

- [ ] **Step 3: Add the fields**

In `src-tauri/src/mcp/config.rs`, immediately after the `image_proxy_allowlist` field (before the closing `}` of `struct Settings`, ~line 242):

```rust
    /// Master toggle for importing instruction skills from Claude Code's
    /// `~/.claude/skills/`. Default false — while off, Nebula never scans or
    /// watches `~/.claude`.
    #[serde(default = "default_false_bool")]
    pub import_claude_skills: bool,

    /// Per-skill overrides for imported Claude skills, keyed by skill slug
    /// (directory name). Presence = explicit user choice; absence = use the
    /// classification heuristic's default. Stored as deviations only so the map
    /// survives skills appearing/disappearing and heuristic changes.
    #[serde(default)]
    pub claude_skill_overrides: std::collections::HashMap<String, bool>,
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib mcp::config::tests::settings_default_claude_import_fields mcp::config::tests::settings_roundtrip_claude_import_fields`
Expected: PASS (2 passed).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/mcp/config.rs
git commit -m "$(cat <<'EOF'
feat(skills): add settings for Claude skill import toggle + overrides

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Data model — `SkillOrigin`, `origin` field, `ClaudeSkillEntry`

**Files:**
- Modify: `src-tauri/src/skills/api.rs` (add `SkillOrigin`, add `origin` to `Skill` + `SkillSummary`, add `ClaudeSkillEntry`)
- Modify: `src-tauri/src/skills/store.rs:79-87` (set `origin: SkillOrigin::Native` in `read_skill`)
- Modify: `src-tauri/src/skills/mod.rs:24` (re-export the new types)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub enum SkillOrigin { Native, Claude }` (serde default `Native`).
  - `Skill.origin: SkillOrigin`, `SkillSummary.origin: SkillOrigin`.
  - `pub struct ClaudeSkillEntry { slug: String, name: String, description: String, heuristic_default: bool, effective_enabled: bool, shadowed_by_native: bool }` (all `pub`), consumed by Tasks 5/6/7.

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/src/skills/store.rs` `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn read_skill_defaults_origin_to_native() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("x.md");
        fs::write(&path, "---\ndescription: d\n---\nbody\n").unwrap();
        let s = read_skill(&path, false).unwrap();
        assert_eq!(s.origin, crate::skills::SkillOrigin::Native);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test --lib skills::store::tests::read_skill_defaults_origin_to_native`
Expected: FAIL to compile — `no variant or associated item named 'Native'` / `no field 'origin'`.

- [ ] **Step 3: Add the types and the `origin` field**

In `src-tauri/src/skills/api.rs`, replace the file's type definitions with:

```rust
//! Skill types exposed to the frontend and the LLM dispatch layer.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where a skill came from. Native = authored in Nebula (user dir or built-in).
/// Claude = imported read-only from Claude Code's `~/.claude/skills/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SkillOrigin {
    #[default]
    Native,
    Claude,
}

/// A fully-loaded skill: identity, description, and the markdown body that
/// gets injected when the LLM calls `use_skill`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub body: String,
    pub built_in: bool,
    pub path: PathBuf,
    #[serde(default)]
    pub origin: SkillOrigin,
}

/// Lightweight view returned to the frontend's Skills tab and used in the
/// system-prompt rendering. Excludes the full body to keep responses small.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub built_in: bool,
    pub path: String,
    #[serde(default)]
    pub origin: SkillOrigin,
}

impl From<&Skill> for SkillSummary {
    fn from(s: &Skill) -> Self {
        Self {
            slug: s.slug.clone(),
            name: s.name.clone(),
            description: s.description.clone(),
            built_in: s.built_in,
            path: s.path.to_string_lossy().into_owned(),
            origin: s.origin,
        }
    }
}

/// One discovered Claude skill plus its computed state, for the Settings
/// checklist. `effective_enabled` is whether it is active in the cache right
/// now (`!shadowed && (override or heuristic_default)`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeSkillEntry {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub heuristic_default: bool,
    pub effective_enabled: bool,
    pub shadowed_by_native: bool,
}
```

In `src-tauri/src/skills/store.rs`, in `read_skill`, set the origin on the returned `Skill` (the struct literal at lines 79-87):

```rust
    Ok(Skill {
        slug,
        name,
        description: front.description,
        body: body.trim_start_matches('\n').to_string(),
        built_in: built_in_flag || front.built_in,
        path: path.to_path_buf(),
        origin: super::api::SkillOrigin::Native,
    })
```

In `src-tauri/src/skills/mod.rs`, widen the re-export at line 24:

```rust
pub use api::{ClaudeSkillEntry, Skill, SkillOrigin, SkillSummary};
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test --lib skills::store::tests::read_skill_defaults_origin_to_native`
Expected: PASS. (Existing `store.rs` tests still compile — they build `Skill` via `read_skill`/`write_skill`, not struct literals.)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/skills/api.rs src-tauri/src/skills/store.rs src-tauri/src/skills/mod.rs
git commit -m "$(cat <<'EOF'
feat(skills): add SkillOrigin + ClaudeSkillEntry data model

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Discover Claude skills from `~/.claude/skills/`

**Files:**
- Modify: `src-tauri/src/skills/store.rs` (add `scan_claude_skills`, `read_claude_skill`, `allowed-tools` parsing; tests)

**Interfaces:**
- Consumes: `super::api::{Skill, SkillOrigin}`, `super::is_valid_slug` (private fn in `mod.rs`, visible to this descendant module), existing `split_frontmatter`.
- Produces: `pub fn scan_claude_skills(dir: &Path) -> Vec<(Skill, Vec<String>)>` — each entry is a parsed Claude skill plus its normalized `allowed-tools` token list (used by Task 4's heuristic and Task 5's reload).

- [ ] **Step 1: Write the failing tests**

Add to `src-tauri/src/skills/store.rs` `mod tests`:

```rust
    fn write_claude_skill(root: &Path, name: &str, frontmatter: &str, body: &str) {
        let d = root.join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("SKILL.md"), format!("---\n{frontmatter}\n---\n{body}\n")).unwrap();
    }

    #[test]
    fn scan_claude_skills_reads_dir_based_skill() {
        let root = TempDir::new().unwrap();
        write_claude_skill(root.path(), "brainstorming", "name: Brainstorming\ndescription: explore ideas", "Do the thing.");
        let found = scan_claude_skills(root.path());
        assert_eq!(found.len(), 1);
        let (skill, tools) = &found[0];
        assert_eq!(skill.slug, "brainstorming");
        assert_eq!(skill.name, "Brainstorming");
        assert_eq!(skill.description, "explore ideas");
        assert!(skill.body.contains("Do the thing"));
        assert_eq!(skill.origin, crate::skills::SkillOrigin::Claude);
        assert!(!skill.built_in);
        assert!(tools.is_empty());
    }

    #[test]
    fn scan_claude_skills_skips_dirs_without_skill_md() {
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join("empty")).unwrap();
        write_claude_skill(root.path(), "ok", "description: d", "b");
        let found = scan_claude_skills(root.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0.slug, "ok");
    }

    #[test]
    fn scan_claude_skills_skips_missing_description() {
        let root = TempDir::new().unwrap();
        write_claude_skill(root.path(), "nodesc", "name: No Desc", "b");
        assert!(scan_claude_skills(root.path()).is_empty());
    }

    #[test]
    fn scan_claude_skills_parses_allowed_tools_list_and_csv() {
        let root = TempDir::new().unwrap();
        write_claude_skill(root.path(), "listy", "description: d\nallowed-tools:\n  - Read\n  - Bash", "b");
        write_claude_skill(root.path(), "csvy", "description: d\nallowed-tools: Read, Bash", "b");
        let mut found = scan_claude_skills(root.path());
        found.sort_by(|a, b| a.0.slug.cmp(&b.0.slug));
        assert_eq!(found[0].1, vec!["Read".to_string(), "Bash".to_string()]); // csvy
        assert_eq!(found[1].1, vec!["Read".to_string(), "Bash".to_string()]); // listy
    }

    #[test]
    fn scan_claude_skills_missing_dir_is_empty() {
        let root = TempDir::new().unwrap();
        let missing = root.path().join("nope");
        assert!(scan_claude_skills(&missing).is_empty());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib skills::store::tests::scan_claude_skills`
Expected: FAIL to compile — `cannot find function 'scan_claude_skills'`.

- [ ] **Step 3: Implement discovery + allowed-tools parsing**

Add to `src-tauri/src/skills/store.rs` (after the existing `read_skill` function). Note `Path`/`PathBuf` and `fs` are already imported at the top of the file:

```rust
/// Accepts a YAML scalar string OR a sequence of strings (Claude's
/// `allowed-tools` appears in both shapes across skills).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum StringOrVec {
    One(String),
    Many(Vec<String>),
}

fn normalize_tools(v: Option<StringOrVec>) -> Vec<String> {
    match v {
        None => Vec::new(),
        Some(StringOrVec::One(s)) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        Some(StringOrVec::Many(xs)) => xs
            .into_iter()
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ClaudeFrontmatter {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "allowed-tools")]
    allowed_tools: Option<StringOrVec>,
}

/// Parse one `<dir>/SKILL.md` into a `Skill` (origin = Claude) plus its
/// normalized `allowed-tools` list. `slug` is the directory name and must be a
/// valid slug.
pub fn read_claude_skill(skill_dir: &Path) -> Result<(Skill, Vec<String>)> {
    let slug = skill_dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("non-utf8 dir name {}", skill_dir.display()))?
        .to_string();
    if !super::is_valid_slug(&slug) {
        return Err(anyhow!("invalid slug from dir name '{slug}'"));
    }

    let md_path = skill_dir.join("SKILL.md");
    let raw = fs::read_to_string(&md_path).with_context(|| format!("read {}", md_path.display()))?;
    let (front_str, body) = split_frontmatter(&raw);
    let front: ClaudeFrontmatter = if front_str.trim().is_empty() {
        ClaudeFrontmatter::default()
    } else {
        serde_yaml::from_str(front_str)
            .with_context(|| format!("parse frontmatter in {}", md_path.display()))?
    };

    if front.description.trim().is_empty() {
        return Err(anyhow!("claude skill {slug} is missing a description"));
    }
    let name = if front.name.is_empty() {
        slug.clone()
    } else {
        front.name.clone()
    };
    let allowed_tools = normalize_tools(front.allowed_tools);

    let skill = Skill {
        slug,
        name,
        description: front.description,
        body: body.trim_start_matches('\n').to_string(),
        built_in: false,
        path: md_path,
        origin: super::api::SkillOrigin::Claude,
    };
    Ok((skill, allowed_tools))
}

/// Scan `dir` (e.g. `~/.claude/skills/`) for `<name>/SKILL.md` skills. Follows
/// symlinks via canonicalize. Unparseable entries are logged and skipped so a
/// single bad skill can't break discovery. Returns each skill with its
/// `allowed-tools` list.
pub fn scan_claude_skills(dir: &Path) -> Vec<(Skill, Vec<String>)> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out, // missing/unreadable dir → empty, non-fatal
    };
    for entry in entries.flatten() {
        // Canonicalize to resolve symlinks (most of ~/.claude/skills are links).
        let path = match fs::canonicalize(entry.path()) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("skip claude skill {}: {e}", entry.path().display());
                continue;
            }
        };
        if !path.is_dir() || !path.join("SKILL.md").is_file() {
            continue;
        }
        match read_claude_skill(&path) {
            Ok(found) => out.push(found),
            Err(e) => tracing::warn!("skip claude skill {}: {e}", path.display()),
        }
    }
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib skills::store::tests::scan_claude_skills`
Expected: PASS (5 passed).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/skills/store.rs
git commit -m "$(cat <<'EOF'
feat(skills): discover dir-based Claude skills from ~/.claude/skills

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Classification heuristic

**Files:**
- Modify: `src-tauri/src/skills/store.rs` (add `claude_skill_default_enabled`; tests)

**Interfaces:**
- Consumes: nothing (pure function over `&str` body + `&[String]` tools).
- Produces: `pub fn claude_skill_default_enabled(body: &str, allowed_tools: &[String]) -> bool` — `true` = default ON (looks like self-sufficient instructions), `false` = default OFF (looks like a script/subagent doer). Consumed by Task 5.

- [ ] **Step 1: Write the failing tests**

Add to `src-tauri/src/skills/store.rs` `mod tests`:

```rust
    #[test]
    fn heuristic_instruction_skill_defaults_on() {
        let body = "Help the user explore ideas one question at a time. Present a design and get approval.";
        assert!(claude_skill_default_enabled(body, &[]));
    }

    #[test]
    fn heuristic_exec_allowed_tools_defaults_off() {
        assert!(!claude_skill_default_enabled("guidance", &["Read".into(), "Bash".into()]));
        assert!(!claude_skill_default_enabled("guidance", &["Write".into()]));
        assert!(!claude_skill_default_enabled("guidance", &["Edit".into()]));
    }

    #[test]
    fn heuristic_script_launcher_defaults_off() {
        let body = "Run the sanitize.py script: `python sanitize.py input.txt`.";
        assert!(!claude_skill_default_enabled(body, &[]));
    }

    #[test]
    fn heuristic_subagent_skill_defaults_off() {
        let body = "First, dispatch a subagent to gather context, then synthesize.";
        assert!(!claude_skill_default_enabled(body, &[]));
    }

    #[test]
    fn heuristic_bare_scripts_dir_mention_stays_on() {
        // Mentioning a .md companion or having a scripts/ dir is NOT a run
        // imperative — must stay ON (this is the `brainstorming` case).
        let body = "If they accept, read visual-companion.md for the detailed guide.";
        assert!(claude_skill_default_enabled(body, &[]));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib skills::store::tests::heuristic`
Expected: FAIL to compile — `cannot find function 'claude_skill_default_enabled'`.

- [ ] **Step 3: Implement the heuristic**

Add to `src-tauri/src/skills/store.rs`:

```rust
/// Best-effort classification of a Claude skill as a self-sufficient
/// instruction skill (default ON) vs a script/subagent launcher Nebula can't
/// run (default OFF). Deliberately conservative toward inclusion: only a strong
/// "this is a doer" signal flips it off. The Settings checklist is the user's
/// authoritative override.
pub fn claude_skill_default_enabled(body: &str, allowed_tools: &[String]) -> bool {
    const EXEC_TOOLS: [&str; 5] = ["bash", "write", "edit", "shell", "execute"];
    if allowed_tools.iter().any(|t| {
        let t = t.to_ascii_lowercase();
        EXEC_TOOLS.iter().any(|e| t.contains(e))
    }) {
        return false;
    }

    let lower = body.to_lowercase();

    const SUBAGENT_MARKERS: [&str; 4] =
        ["dispatch a subagent", "spawn ", "task tool", "subagent"];
    if SUBAGENT_MARKERS.iter().any(|m| lower.contains(m)) {
        return false;
    }

    // Script-execution imperative = a run verb AND a script extension present.
    const RUN_VERBS: [&str; 4] = ["run ", "python ", "bash ", "./"];
    const SCRIPT_EXT: [&str; 3] = [".py", ".sh", ".js"];
    let mentions_script = SCRIPT_EXT.iter().any(|x| lower.contains(x));
    let has_run_verb = RUN_VERBS.iter().any(|v| lower.contains(v));
    if mentions_script && has_run_verb {
        return false;
    }

    true
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib skills::store::tests::heuristic`
Expected: PASS (5 passed).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/skills/store.rs
git commit -m "$(cat <<'EOF'
feat(skills): classify Claude skills as instruction vs doer

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: SkillStore integration — config, reload, collisions, overrides

**Files:**
- Modify: `src-tauri/src/skills/mod.rs` (add `ClaudeImportConfig`, new `SkillStore` fields, `apply_claude_import`, `set_claude_import`, `list_claude_skills`; extend `reload`; store reload callback in `start_watcher`; tests)

**Interfaces:**
- Consumes: `store::scan_claude_skills`, `store::claude_skill_default_enabled` (Tasks 3/4); `ClaudeSkillEntry` (Task 2); existing `store::scan_all`, `watcher::start_watching`.
- Produces:
  - `pub struct ClaudeImportConfig { pub enabled: bool, pub dir: PathBuf, pub overrides: HashMap<String, bool> }` (`Clone, Default, PartialEq`).
  - `SkillStore::set_claude_import(self: &Arc<Self>, cfg: ClaudeImportConfig) -> Result<()>` (sets config, re-arms/drops the `~/.claude/skills` watcher, reloads) — called by Task 6.
  - `SkillStore::list_claude_skills(&self) -> Vec<ClaudeSkillEntry>` — called by Task 6.

- [ ] **Step 1: Write the failing tests**

Add a test module at the end of `src-tauri/src/skills/mod.rs`:

```rust
#[cfg(test)]
mod claude_import_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_claude(root: &std::path::Path, name: &str, front: &str, body: &str) {
        let d = root.join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("SKILL.md"), format!("---\n{front}\n---\n{body}\n")).unwrap();
    }

    async fn store_with(skills_dir: TempDir) -> Arc<SkillStore> {
        SkillStore::new(skills_dir.path().to_path_buf()).await.unwrap()
    }

    #[tokio::test]
    async fn import_disabled_surfaces_nothing() {
        let skills = TempDir::new().unwrap();
        let claude = TempDir::new().unwrap();
        write_claude(claude.path(), "advice", "description: d", "be wise");
        let store = store_with(skills).await;
        store
            .apply_claude_import(ClaudeImportConfig {
                enabled: false,
                dir: claude.path().to_path_buf(),
                overrides: HashMap::new(),
            })
            .await
            .unwrap();
        assert!(store.list_claude_skills().await.is_empty());
        assert!(store.get("advice").await.is_none());
    }

    #[tokio::test]
    async fn enabled_imports_instruction_skill_into_cache() {
        let skills = TempDir::new().unwrap();
        let claude = TempDir::new().unwrap();
        write_claude(claude.path(), "advice", "name: Advice\ndescription: d", "be wise");
        let store = store_with(skills).await;
        store
            .apply_claude_import(ClaudeImportConfig {
                enabled: true,
                dir: claude.path().to_path_buf(),
                overrides: HashMap::new(),
            })
            .await
            .unwrap();

        let entries = store.list_claude_skills().await;
        assert_eq!(entries.len(), 1);
        assert!(entries[0].heuristic_default);
        assert!(entries[0].effective_enabled);
        assert!(!entries[0].shadowed_by_native);

        let loaded = store.get("advice").await.unwrap();
        assert_eq!(loaded.origin, SkillOrigin::Claude);
    }

    #[tokio::test]
    async fn doer_skill_off_by_default_but_overridable_on() {
        let skills = TempDir::new().unwrap();
        let claude = TempDir::new().unwrap();
        write_claude(claude.path(), "doer", "description: d", "dispatch a subagent to do it");
        let store = store_with(skills).await;

        store
            .apply_claude_import(ClaudeImportConfig {
                enabled: true,
                dir: claude.path().to_path_buf(),
                overrides: HashMap::new(),
            })
            .await
            .unwrap();
        let e = &store.list_claude_skills().await[0];
        assert!(!e.heuristic_default);
        assert!(!e.effective_enabled);
        assert!(store.get("doer").await.is_none());

        let mut overrides = HashMap::new();
        overrides.insert("doer".to_string(), true);
        store
            .apply_claude_import(ClaudeImportConfig {
                enabled: true,
                dir: claude.path().to_path_buf(),
                overrides,
            })
            .await
            .unwrap();
        assert!(store.list_claude_skills().await[0].effective_enabled);
        assert!(store.get("doer").await.is_some());
    }

    #[tokio::test]
    async fn native_shadows_claude_slug() {
        let skills = TempDir::new().unwrap();
        // Native user skill `advice`.
        crate::skills::store::write_skill(
            &skills.path().join("advice.md"),
            "advice",
            "Native Advice",
            "native desc",
            "native body",
            false,
        )
        .unwrap();
        let claude = TempDir::new().unwrap();
        write_claude(claude.path(), "advice", "description: claude desc", "claude body");
        let store = store_with(skills).await;
        store
            .apply_claude_import(ClaudeImportConfig {
                enabled: true,
                dir: claude.path().to_path_buf(),
                overrides: HashMap::new(),
            })
            .await
            .unwrap();

        let e = &store.list_claude_skills().await[0];
        assert!(e.shadowed_by_native);
        assert!(!e.effective_enabled);
        // The active `advice` is the native one.
        let loaded = store.get("advice").await.unwrap();
        assert_eq!(loaded.origin, SkillOrigin::Native);
        assert_eq!(loaded.body, "native body");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test --lib skills::claude_import_tests`
Expected: FAIL to compile — `cannot find type 'ClaudeImportConfig'` / `no method 'apply_claude_import'`.

- [ ] **Step 3: Add config struct, fields, and methods**

In `src-tauri/src/skills/mod.rs`:

(a) Extend imports at the top — add `HashMap`:

```rust
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::RwLock;
```

(b) Add the config type (after the `pub use` line):

```rust
/// What `SkillStore` needs to know to import Claude skills. Built by `lib.rs`
/// from `Settings` + the resolved `~/.claude/skills` path. The skills module
/// stays unaware of settings.json.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClaudeImportConfig {
    pub enabled: bool,
    pub dir: PathBuf,
    pub overrides: HashMap<String, bool>,
}

type ReloadCb = Arc<dyn Fn() + Send + Sync>;
```

(c) Replace the `SkillStore` struct definition with:

```rust
pub struct SkillStore {
    skills_dir: PathBuf,
    cache: Arc<RwLock<Vec<Skill>>>,
    watcher: StdMutex<Option<watcher::SkillsWatcher>>,
    claude_watcher: StdMutex<Option<watcher::SkillsWatcher>>,
    claude_import: RwLock<ClaudeImportConfig>,
    claude_discovered: RwLock<Vec<ClaudeSkillEntry>>,
    reload_cb: StdMutex<Option<ReloadCb>>,
}
```

(d) In `SkillStore::new`, replace the `Self { ... }` initializer (lines ~52-56) with:

```rust
        let me = Arc::new(Self {
            skills_dir,
            cache: Arc::new(RwLock::new(Vec::new())),
            watcher: StdMutex::new(None),
            claude_watcher: StdMutex::new(None),
            claude_import: RwLock::new(ClaudeImportConfig::default()),
            claude_discovered: RwLock::new(Vec::new()),
            reload_cb: StdMutex::new(None),
        });
```

(e) In `start_watcher`, store the callback so the Claude watcher can reuse it. Change the start of the method body to capture into `reload_cb`:

```rust
    pub fn start_watcher<F>(self: &Arc<Self>, on_reload: F) -> Result<()>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let on_reload: ReloadCb = Arc::new(on_reload);
        *self.reload_cb.lock().expect("reload cb poisoned") = Some(on_reload.clone());
        let me = self.clone();
        let handle = watcher::start_watching(self.skills_dir.clone(), move || {
            let me = me.clone();
            let on_reload = on_reload.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = me.reload().await {
                    tracing::warn!("skills reload (watcher) failed: {e}");
                } else {
                    on_reload();
                }
            });
        })?;
        *self.watcher.lock().expect("skills watcher slot poisoned") = Some(handle);
        Ok(())
    }
```

(f) Replace the existing `reload` method with the merged version:

```rust
    /// Re-scan native skills, then (when import is enabled) discover, classify,
    /// and merge Claude skills. Populates both the active cache (native +
    /// enabled, non-shadowed Claude) and the discovered list (every Claude
    /// skill with computed state) for the Settings checklist.
    pub async fn reload(&self) -> Result<()> {
        let mut skills = store::scan_all(&self.skills_dir)?;
        let native_slugs: std::collections::HashSet<String> =
            skills.iter().map(|s| s.slug.clone()).collect();

        let cfg = self.claude_import.read().await.clone();
        let mut discovered: Vec<ClaudeSkillEntry> = Vec::new();
        if cfg.enabled {
            for (skill, allowed_tools) in store::scan_claude_skills(&cfg.dir) {
                let shadowed = native_slugs.contains(&skill.slug);
                let heuristic_default =
                    store::claude_skill_default_enabled(&skill.body, &allowed_tools);
                let resolved = cfg
                    .overrides
                    .get(&skill.slug)
                    .copied()
                    .unwrap_or(heuristic_default);
                let effective_enabled = !shadowed && resolved;
                discovered.push(ClaudeSkillEntry {
                    slug: skill.slug.clone(),
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    heuristic_default,
                    effective_enabled,
                    shadowed_by_native: shadowed,
                });
                if effective_enabled {
                    skills.push(skill);
                }
            }
        }

        skills.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        discovered.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        *self.cache.write().await = skills;
        *self.claude_discovered.write().await = discovered;
        Ok(())
    }
```

(g) Add the import-control methods (place after `reload`):

```rust
    /// The discovered Claude skills with their computed state, for the Settings
    /// checklist. Empty when import is disabled.
    pub async fn list_claude_skills(&self) -> Vec<ClaudeSkillEntry> {
        self.claude_discovered.read().await.clone()
    }

    /// Set the import config and reload. Does NOT touch the watcher — used
    /// internally and by tests. `set_claude_import` wraps this with watcher
    /// management.
    pub(crate) async fn apply_claude_import(&self, cfg: ClaudeImportConfig) -> Result<()> {
        *self.claude_import.write().await = cfg;
        self.reload().await
    }

    /// Public entry point used by `lib.rs`: apply the config, then (re)arm or
    /// drop the `~/.claude/skills` watcher to match. No-op when the config is
    /// unchanged.
    pub async fn set_claude_import(self: &Arc<Self>, cfg: ClaudeImportConfig) -> Result<()> {
        {
            if *self.claude_import.read().await == cfg {
                return Ok(());
            }
        }
        let enabled = cfg.enabled;
        let dir = cfg.dir.clone();
        self.apply_claude_import(cfg).await?;

        let mut slot = self
            .claude_watcher
            .lock()
            .expect("claude watcher slot poisoned");
        *slot = None; // drop any existing watcher first
        if enabled && dir.exists() {
            let me = self.clone();
            let cb = self.reload_cb.lock().expect("reload cb poisoned").clone();
            match watcher::start_watching(dir, move || {
                let me = me.clone();
                let cb = cb.clone();
                tauri::async_runtime::spawn(async move {
                    if me.reload().await.is_ok() {
                        if let Some(cb) = &cb {
                            cb();
                        }
                    }
                });
            }) {
                Ok(h) => *slot = Some(h),
                Err(e) => tracing::warn!("claude skills watcher failed to start: {e}"),
            }
        }
        Ok(())
    }
```

(h) The re-export line must already expose `ClaudeImportConfig`. Update it:

```rust
pub use api::{ClaudeSkillEntry, Skill, SkillOrigin, SkillSummary};
```

(`ClaudeImportConfig` is defined in `mod.rs` itself, so it is already `crate::skills::ClaudeImportConfig`.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib skills::claude_import_tests`
Expected: PASS (4 passed).

- [ ] **Step 5: Run the whole skills + config suite to catch regressions**

Run: `cd src-tauri && cargo test --lib skills:: mcp::config::tests`
Expected: PASS (all existing skills/store/config tests + the new ones).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/skills/mod.rs
git commit -m "$(cat <<'EOF'
feat(skills): merge enabled Claude skills into SkillStore (C1 + overrides)

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: Tauri wiring — startup, command, save_settings

**Files:**
- Modify: `src-tauri/src/lib.rs` (startup `set_claude_import`; new `list_claude_skills` command + registration; `save_settings` gains `State` + pushes config + emits `skills-updated`)

**Interfaces:**
- Consumes: `crate::skills::{ClaudeImportConfig, ClaudeSkillEntry}` (Tasks 2/5), `SkillStore::{set_claude_import, list_claude_skills}` (Task 5), `Settings.{import_claude_skills, claude_skill_overrides}` (Task 1).
- Produces: Tauri command `list_claude_skills` callable from the frontend (Task 7).

There is no Rust unit test here (Tauri command + setup wiring). Verification is `cargo build` + manual smoke test.

- [ ] **Step 1: Add a helper to build the import config from settings**

In `src-tauri/src/lib.rs`, add near the other skills commands (after `reload_skills`, ~line 1601):

```rust
/// Resolve `~/.claude/skills` and fold the relevant settings into a
/// `ClaudeImportConfig`. Returns `None` only if the home dir can't be resolved.
fn build_claude_import_config(
    app: &tauri::AppHandle,
    settings: &Settings,
) -> Option<crate::skills::ClaudeImportConfig> {
    let home = app.path().home_dir().ok()?;
    Some(crate::skills::ClaudeImportConfig {
        enabled: settings.import_claude_skills,
        dir: home.join(".claude").join("skills"),
        overrides: settings.claude_skill_overrides.clone(),
    })
}

#[tauri::command]
async fn list_claude_skills(
    state: State<'_, AppState>,
) -> Result<Vec<crate::skills::ClaudeSkillEntry>, String> {
    Ok(state.skills.list_claude_skills().await)
}
```

- [ ] **Step 2: Wire startup import after the watcher is armed**

In the setup block of `src-tauri/src/lib.rs`, immediately after the `start_watcher` block (after line 3439, before `// 4. AppState`), add:

```rust
                // Apply the Claude-skills import config from settings (if the
                // toggle is on). Must run AFTER start_watcher so the reload
                // callback is registered for the ~/.claude/skills watcher.
                {
                    let settings_path = config_dir.join("settings.json");
                    let settings = Settings::load_migrated(&settings_path);
                    if let Some(cfg) = build_claude_import_config(&app_handle, &settings) {
                        if let Err(e) = skills_store.set_claude_import(cfg).await {
                            tracing::warn!("claude skills import failed at startup: {e}");
                        }
                    }
                }
```

- [ ] **Step 3: Update `save_settings` to push the config and refresh the UI**

In `src-tauri/src/lib.rs`, replace the `save_settings` command (lines 2131-2141) with:

```rust
#[tauri::command]
async fn save_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let settings_path = config_dir.join("settings.json");
    settings.save(&settings_path).map_err(|e| e.to_string())?;

    use tauri::Emitter;
    app.emit("settings-updated", ()).map_err(|e| e.to_string())?;

    // Reflect any change to the Claude-skills import toggle/overrides, then
    // refresh the skills UI. set_claude_import is a no-op when unchanged.
    if let Some(cfg) = build_claude_import_config(&app, &settings) {
        state
            .skills
            .set_claude_import(cfg)
            .await
            .map_err(|e| e.to_string())?;
        app.emit("skills-updated", ()).map_err(|e| e.to_string())?;
    }
    Ok(())
}
```

- [ ] **Step 4: Register the new command**

In `src-tauri/src/lib.rs`, in the `invoke_handler![...]` list, add `list_claude_skills` after `reload_skills` (line ~3614):

```rust
            reload_skills,
            list_claude_skills
        ])
```

- [ ] **Step 5: Build to verify the backend compiles**

Run: `cd src-tauri && cargo build`
Expected: compiles cleanly (warnings ok, no errors).

- [ ] **Step 6: Run the full backend test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS (no regressions).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(skills): wire Claude skill import into startup, command, save_settings

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Frontend — master toggle + read-only checklist

**Files:**
- Modify: `src/components/SkillsSettings.tsx` (add `ClaudeSkillEntry` type, settings load/save for the toggle + overrides, the toggle UI, the checklist; refresh on `skills-updated`)

**Interfaces:**
- Consumes: Tauri commands `get_settings`, `save_settings`, `list_claude_skills`; event `skills-updated`.
- Produces: UI only.

No automated test (no frontend test suite per CLAUDE.md). Verification is `npm run build` + manual smoke test via `npm run tauri dev`.

- [ ] **Step 1: Add the type and state**

In `src/components/SkillsSettings.tsx`, after the `Skill` interface (line 16), add:

```tsx
interface ClaudeSkillEntry {
    slug: string;
    name: string;
    description: string;
    heuristic_default: boolean;
    effective_enabled: boolean;
    shadowed_by_native: boolean;
}

// Minimal shape of the parts of Settings this component reads/writes. The
// backend's save_settings takes the full Settings object, so we load it whole,
// mutate these fields, and save it back to avoid clobbering other settings.
type Settings = Record<string, unknown> & {
    import_claude_skills?: boolean;
    claude_skill_overrides?: Record<string, boolean>;
};
```

Then add state inside the component (after the `status` state, line 27):

```tsx
    const [importClaude, setImportClaude] = useState(false);
    const [claudeSkills, setClaudeSkills] = useState<ClaudeSkillEntry[]>([]);
```

- [ ] **Step 2: Load the import state and refresh on changes**

In `src/components/SkillsSettings.tsx`, add a loader and call it alongside `loadSkills`. Add after the `loadSkills` definition (line 36):

```tsx
    const loadClaudeImport = async () => {
        try {
            const settings = await invoke<Settings>("get_settings");
            setImportClaude(!!settings.import_claude_skills);
            if (settings.import_claude_skills) {
                setClaudeSkills(await invoke<ClaudeSkillEntry[]>("list_claude_skills"));
            } else {
                setClaudeSkills([]);
            }
        } catch (e) {
            console.error(e);
        }
    };
```

Update the `useEffect` (lines 38-49) to also load and refresh the import state:

```tsx
    useEffect(() => {
        loadSkills();
        loadClaudeImport();
        // The backend FS watcher emits `skills-updated` whenever a file under
        // skills/ (or, when enabled, ~/.claude/skills) changes, and after the
        // import toggle/overrides are saved. Refresh both views.
        const unlisten = listen("skills-updated", () => {
            loadSkills();
            loadClaudeImport();
        });
        return () => {
            unlisten.then((fn) => fn());
        };
    }, []);
```

- [ ] **Step 3: Add the toggle + override handlers**

In `src/components/SkillsSettings.tsx`, add after `loadClaudeImport` (or near the other handlers):

```tsx
    const persistSettings = async (mutate: (s: Settings) => void) => {
        const settings = await invoke<Settings>("get_settings");
        mutate(settings);
        await invoke("save_settings", { settings });
        // save_settings emits skills-updated, which triggers loadClaudeImport.
    };

    const handleToggleImport = async () => {
        const next = !importClaude;
        setImportClaude(next); // optimistic
        try {
            await persistSettings((s) => {
                s.import_claude_skills = next;
            });
        } catch (e) {
            console.error(e);
            setImportClaude(!next); // revert on failure
        }
    };

    const handleToggleClaudeSkill = async (entry: ClaudeSkillEntry) => {
        if (entry.shadowed_by_native) return;
        const next = !entry.effective_enabled;
        try {
            await persistSettings((s) => {
                const overrides = { ...(s.claude_skill_overrides ?? {}) };
                overrides[entry.slug] = next;
                s.claude_skill_overrides = overrides;
            });
        } catch (e) {
            console.error(e);
        }
    };
```

- [ ] **Step 4: Render the toggle + checklist**

In `src/components/SkillsSettings.tsx`, the component currently returns a single `<div className="flex h-[500px] ...">`. Wrap the return in a fragment with a new section above it. Replace `return (` ... up to that opening `<div className="flex h-[500px] ...">` with:

```tsx
    return (
      <div className="space-y-4">
        {/* Claude Code skills import */}
        <div className="border border-[var(--color-border-primary)] rounded-xl bg-[var(--color-bg-secondary)] p-4 space-y-3">
            <label className="flex items-center justify-between gap-3 cursor-pointer">
                <span>
                    <span className="font-bold text-sm text-[var(--color-text-primary)]">
                        Import skills from Claude Code
                    </span>
                    <span className="block text-xs text-[var(--color-text-tertiary)] font-mono">
                        ~/.claude/skills
                    </span>
                </span>
                <input
                    type="checkbox"
                    checked={importClaude}
                    onChange={handleToggleImport}
                    className="h-4 w-4 shrink-0"
                />
            </label>

            {importClaude && (
                <div className="space-y-1 max-h-48 overflow-y-auto pt-1 border-t border-[var(--color-border-secondary)]">
                    {claudeSkills.length === 0 && (
                        <div className="text-center text-[var(--color-text-tertiary)] text-xs py-6">
                            No Claude skills found in ~/.claude/skills.
                        </div>
                    )}
                    {claudeSkills.map((c) => (
                        <label
                            key={c.slug}
                            className={`flex items-start gap-2 p-2 rounded text-sm ${
                                c.shadowed_by_native ? "opacity-50" : "hover:bg-[var(--color-bg-tertiary)] cursor-pointer"
                            }`}
                        >
                            <input
                                type="checkbox"
                                checked={c.effective_enabled}
                                disabled={c.shadowed_by_native}
                                onChange={() => handleToggleClaudeSkill(c)}
                                className="h-4 w-4 mt-0.5 shrink-0"
                            />
                            <span className="min-w-0">
                                <span className="flex items-center gap-2">
                                    <span className="truncate text-[var(--color-text-secondary)]">{c.name}</span>
                                    <span className="shrink-0 text-[9px] uppercase tracking-wider bg-blue-500/20 text-blue-300 px-1 rounded">
                                        from Claude Code
                                    </span>
                                </span>
                                <span className="block text-xs text-[var(--color-text-tertiary)] truncate">
                                    {c.shadowed_by_native
                                        ? "Shadowed by a native skill"
                                        : !c.heuristic_default && !c.effective_enabled
                                        ? "Looks like it needs scripts — off by default"
                                        : c.description}
                                </span>
                            </span>
                        </label>
                    ))}
                </div>
            )}
        </div>

        <div className="flex h-[500px] border border-[var(--color-border-primary)] rounded-xl overflow-hidden bg-[var(--color-bg-secondary)]">
```

Then add one closing `</div>` at the very end of the JSX: the existing outer `</div>` (line 280) closes the `flex h-[500px]` container; add a final `</div>` after it to close the new `space-y-4` wrapper. The tail becomes:

```tsx
            </div>
          </div>
    );
}
```

- [ ] **Step 5: Build the frontend**

Run: `npm run build`
Expected: TypeScript + Vite build succeeds with no errors.

- [ ] **Step 6: Manual smoke test**

Run: `npm run tauri dev`

Verify:
1. Settings → Skills shows the new **"Import skills from Claude Code"** toggle, off by default; no checklist visible.
2. Toggle on → checklist appears listing skills from `~/.claude/skills`; instruction skills (e.g. `brainstorming`, `systematic-debugging`) checked; obvious doer/subagent skills unchecked with the "needs scripts" note; any slug colliding a native skill shows "Shadowed by a native skill" and a disabled checkbox.
3. Check a previously-off skill → it persists (reopen Settings; still checked) and appears in `list_skills` (ask the model "what skills do you have?" or check the system prompt block).
4. Uncheck an on-by-default skill → it disappears from the active list.
5. Toggle off → checklist hides; imported skills no longer in `list_skills`.
6. With the toggle on, `rm`/add a dir under `~/.claude/skills` → the checklist refreshes within ~1s (watcher).

- [ ] **Step 7: Commit**

```bash
git add src/components/SkillsSettings.tsx
git commit -m "$(cat <<'EOF'
feat(skills): Settings UI to import Claude Code skills (toggle + checklist)

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**1. Spec coverage:**

| Spec requirement | Task |
|---|---|
| `import_claude_skills` toggle (M2) | Task 1, UI in Task 7, startup/save wiring in Task 6 |
| `claude_skill_overrides` store-deviations-only (A3) | Task 1, applied in Task 5, UI in Task 7 |
| `SkillOrigin` on Skill/SkillSummary | Task 2 |
| `ClaudeSkillEntry` for the checklist | Task 2 (type), Task 5 (populated), Task 6 (command), Task 7 (rendered) |
| `scan_claude_skills` dir/SKILL.md, canonicalize symlinks, slug from dir name, capture `allowed-tools` | Task 3 |
| Classification heuristic (allowed-tools / run-script / subagent), conservative toward inclusion | Task 4 |
| Two cached views (active cache + discovered list) | Task 5 |
| C1 native-wins collisions with `shadowed_by_native` | Task 5 (logic + test), Task 7 (display) |
| `set_claude_import` + watcher arm/drop on `~/.claude/skills` | Task 5 (method), Task 6 (called at startup + save) |
| `list_claude_skills` command | Task 6 |
| Startup applies config after watcher | Task 6 |
| Resolve `~/.claude/skills` via home dir, no new deps | Task 6 (`app.path().home_dir()`) |
| Read-only imports (no edit/delete) | Task 7 (checklist is the only control; native editor untouched) |
| Non-fatal errors (skip + warn) | Task 3 (`scan_claude_skills`), inherited by Task 5 |
| Backward-compatible settings/migration | Task 1 (tests) |
| `use_skill`/`list_skills`/`render_for_system_prompt` unchanged | Confirmed — they read `cache`/`list()`, which Task 5 populates; no edits to those paths |

No gaps.

**2. Placeholder scan:** No TBD/TODO/"handle edge cases" — every code step shows complete code, every test shows real assertions, every command shows expected output.

**3. Type consistency:**
- `ClaudeImportConfig { enabled, dir, overrides }` — defined Task 5, built identically in Task 6's `build_claude_import_config`.
- `ClaudeSkillEntry` fields (`slug, name, description, heuristic_default, effective_enabled, shadowed_by_native`) — identical across Task 2 (Rust), Task 5 (populated), Task 7 (TS interface).
- `claude_skill_default_enabled(body: &str, allowed_tools: &[String]) -> bool` — defined Task 4, called Task 5 with `&skill.body, &allowed_tools`.
- `scan_claude_skills(dir) -> Vec<(Skill, Vec<String>)>` — defined Task 3, destructured `for (skill, allowed_tools)` in Task 5.
- `set_claude_import(self: &Arc<Self>, cfg)` / `apply_claude_import(&self, cfg)` — defined Task 5; `set_claude_import` called in Task 6, `apply_claude_import` only in Task 5 tests.
- `list_claude_skills(&self) -> Vec<ClaudeSkillEntry>` — defined Task 5, command-wrapped Task 6, invoked Task 7.

Consistent.
