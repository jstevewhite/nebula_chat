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
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::RwLock;

pub use api::{Skill, SkillSummary};

pub struct SkillStore {
    skills_dir: PathBuf,
    cache: Arc<RwLock<Vec<Skill>>>,
    watcher: StdMutex<Option<watcher::SkillsWatcher>>,
}

impl SkillStore {
    /// Construct the store. Ensures the built-ins are present on disk
    /// (writes them on first run only — user edits stick across launches)
    /// then scans the skills dir into memory.
    pub async fn new(skills_dir: PathBuf) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&skills_dir)?;
        let builtins_dir = skills_dir.join("built-ins");
        std::fs::create_dir_all(&builtins_dir)?;

        // Materialise built-ins on first run only. We compare per-file so a
        // newly-shipped built-in lands even if the dir already exists.
        for (slug, body) in builtins::ALL {
            let path = builtins_dir.join(format!("{slug}.md"));
            if !path.exists() {
                std::fs::write(&path, body)?;
            }
        }

        let me = Arc::new(Self {
            skills_dir,
            cache: Arc::new(RwLock::new(Vec::new())),
            watcher: StdMutex::new(None),
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
        let me = self.clone();
        let on_reload = Arc::new(on_reload);
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

    /// Re-scan the skills dir from disk. Called at startup and after any UI
    /// CRUD operation. We keep the scan synchronous and cheap (handful of
    /// small markdown files); no watcher in v1 — the UI invalidates after
    /// edits.
    pub async fn reload(&self) -> Result<()> {
        let mut skills = store::scan_all(&self.skills_dir)?;
        skills.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        let mut guard = self.cache.write().await;
        *guard = skills;
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
