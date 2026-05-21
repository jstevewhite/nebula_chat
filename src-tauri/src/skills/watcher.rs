//! `notify`-based file watcher for the skills directory. Coalesces FS events
//! on a 250 ms debounce and triggers a single `SkillStore::reload` per burst.
//! Mirrors the pattern in `memory/docs/watcher.rs` but simplified — we only
//! need "something changed, reload all" since the cache is small and the scan
//! is cheap.

use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Handle that keeps the OS watcher alive. Drop it to stop watching.
pub struct SkillsWatcher {
    _watcher: RecommendedWatcher,
    _drain_task: tauri::async_runtime::JoinHandle<()>,
}

/// Start watching `skills_dir` (recursive — covers the `built-ins/` subdir).
/// `on_change` fires at most once per 250 ms debounce window when any `.md`
/// file changes. Returns a handle that must be kept alive.
pub fn start_watching<F>(skills_dir: PathBuf, on_change: F) -> Result<SkillsWatcher>
where
    F: Fn() + Send + Sync + 'static,
{
    std::fs::create_dir_all(&skills_dir)?;

    let pending: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let pending_for_cb = pending.clone();

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            let Ok(event) = res else {
                return;
            };
            if !is_relevant(&event.kind) {
                return;
            }
            if !event.paths.iter().any(|p| is_markdown(p)) {
                return;
            }
            let pending = pending_for_cb.clone();
            tauri::async_runtime::spawn(async move {
                *pending.lock().await = true;
            });
        },
        notify::Config::default(),
    )?;

    watcher.watch(&skills_dir, RecursiveMode::Recursive)?;

    let on_change = Arc::new(on_change);
    let drain_task = tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let fired = {
                let mut guard = pending.lock().await;
                let v = *guard;
                *guard = false;
                v
            };
            if fired {
                on_change();
            }
        }
    });

    Ok(SkillsWatcher {
        _watcher: watcher,
        _drain_task: drain_task,
    })
}

fn is_relevant(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

fn is_markdown(p: &Path) -> bool {
    p.extension().and_then(|s| s.to_str()) == Some("md")
}
