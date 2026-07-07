//! Skills — discoverable, named bundles of imperative instructions the model
//! can pull into context via the `use_skill` LLM tool.
//!
//! Conceptually adjacent to memory docs but with different semantics:
//! - **memory docs** capture factual / historical content
//! - **skills** capture imperative instructions ("how to approach X")
//!
//! Skills live as markdown files on disk under `~/.config/.../skills/<slug>.md`
//! (or `~/.config/.../skills/built-ins/<slug>.md` for the starter set bundled
//! with the app). Frontmatter declares the name + description; the body is the
//! system-prompt-shaped guidance returned when the model calls `use_skill`.

pub mod api;
pub mod builtins;
pub mod store;
pub mod tools;
pub mod watcher;

use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::RwLock;

pub use api::{ClaudeSkillEntry, Skill, SkillOrigin, SkillSummary};

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

pub struct SkillStore {
    skills_dir: PathBuf,
    cache: Arc<RwLock<Vec<Skill>>>,
    watcher: StdMutex<Option<watcher::SkillsWatcher>>,
    claude_watcher: StdMutex<Option<watcher::SkillsWatcher>>,
    claude_import: RwLock<ClaudeImportConfig>,
    claude_discovered: RwLock<Vec<ClaudeSkillEntry>>,
    reload_cb: StdMutex<Option<ReloadCb>>,
}

impl SkillStore {
    /// Construct the store. Ensures the built-ins are present on disk
    /// (writes them on first run only — user edits stick across launches)
    /// then scans the skills dir into memory.
    pub async fn new(skills_dir: PathBuf) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&skills_dir)?;
        let builtins_dir = skills_dir.join("built-ins");
        std::fs::create_dir_all(&builtins_dir)?;

        // Re-materialise built-ins on every startup. `built_in: true` means
        // we own the content — the binary is the source of truth, and any
        // edits to files in `built-ins/` are overwritten on launch. To
        // customise a built-in, create a top-level user skill with the same
        // slug; `get()` finds the user skill first and overrides the
        // built-in.
        for (slug, body) in builtins::ALL {
            let path = builtins_dir.join(format!("{slug}.md"));
            std::fs::write(&path, body)?;
        }

        let me = Arc::new(Self {
            skills_dir,
            cache: Arc::new(RwLock::new(Vec::new())),
            watcher: StdMutex::new(None),
            claude_watcher: StdMutex::new(None),
            claude_import: RwLock::new(ClaudeImportConfig::default()),
            claude_discovered: RwLock::new(Vec::new()),
            reload_cb: StdMutex::new(None),
        });
        me.reload().await?;
        Ok(me)
    }

    pub fn skills_dir(&self) -> &PathBuf {
        &self.skills_dir
    }

    /// Arm the FS watcher. Any change to a `.md` file under the skills dir
    /// (or its `built-ins/` subdir) triggers a debounced reload. `on_reload`
    /// is invoked after each successful reload so the caller (lib.rs) can
    /// emit a Tauri event for the UI.
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

    pub async fn list(&self) -> Vec<SkillSummary> {
        let guard = self.cache.read().await;
        guard.iter().map(SkillSummary::from).collect()
    }

    pub async fn get(&self, slug: &str) -> Option<Skill> {
        let guard = self.cache.read().await;
        guard.iter().find(|s| s.slug == slug).cloned()
    }

    /// Compact "available skills" snippet for the system prompt. Empty when
    /// the user has no skills installed.
    pub async fn render_for_system_prompt(&self) -> Option<String> {
        let guard = self.cache.read().await;
        if guard.is_empty() {
            return None;
        }
        let mut out = String::from(
            "Available skills (call the `use_skill` tool with the name to load):\n",
        );
        for s in guard.iter() {
            out.push_str(&format!("- `{}` — {}\n", s.slug, s.description));
        }
        Some(out)
    }

    /// Create a new user skill. `built_in: true` is rejected — built-ins are
    /// materialised by `new()` from the binary.
    pub async fn create(
        &self,
        slug: &str,
        name: &str,
        description: &str,
        body: &str,
    ) -> Result<()> {
        if !is_valid_slug(slug) {
            anyhow::bail!("invalid slug '{slug}': must match [a-z0-9][a-z0-9-]{{0,63}}");
        }
        let path = self.skills_dir.join(format!("{slug}.md"));
        if path.exists() {
            anyhow::bail!("skill '{slug}' already exists");
        }
        store::write_skill(&path, slug, name, description, body, false)?;
        self.reload().await?;
        Ok(())
    }

    /// Update an existing skill in place. Disallows editing the slug (rename =
    /// delete + create).
    pub async fn update(
        &self,
        slug: &str,
        name: &str,
        description: &str,
        body: &str,
    ) -> Result<()> {
        let existing = self
            .get(slug)
            .await
            .ok_or_else(|| anyhow::anyhow!("skill '{slug}' does not exist"))?;
        store::write_skill(&existing.path, slug, name, description, body, existing.built_in)?;
        self.reload().await?;
        Ok(())
    }

    /// Delete a user skill. Built-ins can be deleted from disk but will be
    /// regenerated on the next app start.
    pub async fn delete(&self, slug: &str) -> Result<()> {
        let existing = self
            .get(slug)
            .await
            .ok_or_else(|| anyhow::anyhow!("skill '{slug}' does not exist"))?;
        std::fs::remove_file(&existing.path)?;
        self.reload().await?;
        Ok(())
    }
}

fn is_valid_slug(s: &str) -> bool {
    if s.is_empty() || s.len() > 64 {
        return false;
    }
    let bytes = s.as_bytes();
    if !(bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit()) {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

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
        // Leak the TempDir so the directory is not deleted when `store_with`
        // returns. Tests that write native skills before calling
        // `apply_claude_import` need the dir to persist across the re-scan.
        let path = skills_dir.path().to_path_buf();
        std::mem::forget(skills_dir);
        SkillStore::new(path).await.unwrap()
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
        assert_eq!(loaded.body.trim_end(), "native body");
    }
}
