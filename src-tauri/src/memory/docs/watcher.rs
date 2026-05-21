//! `notify`-based file watcher for the docs directory. Coalesces FS events on
//! a 250 ms debounce and dispatches one ingestion per changed path.

use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Handle that keeps the OS watcher alive. Drop it to stop watching.
pub struct DocsWatcher {
    _watcher: RecommendedWatcher,
    _drain_task: tauri::async_runtime::JoinHandle<()>,
}

/// Start watching `docs_dir`. For every coalesced `*.md` change, the
/// `on_path` callback is invoked with the absolute path. Returns a handle
/// that must be kept alive to keep the watcher running.
pub fn start_watching<F>(docs_dir: PathBuf, on_path: F) -> Result<DocsWatcher>
where
    F: Fn(PathBuf) + Send + Sync + 'static,
{
    std::fs::create_dir_all(&docs_dir)?;

    let pending: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));
    let pending_for_cb = pending.clone();

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            let Ok(event) = res else {
                return;
            };
            if !is_relevant(&event.kind) {
                return;
            }
            let pending = pending_for_cb.clone();
            // The notify callback runs on its own thread; push into the shared
            // set without blocking the runtime.
            tauri::async_runtime::spawn(async move {
                let mut guard = pending.lock().await;
                for p in event.paths {
                    if is_markdown(&p) {
                        guard.insert(p);
                    }
                }
            });
        },
        notify::Config::default(),
    )?;

    watcher.watch(&docs_dir, RecursiveMode::NonRecursive)?;

    let on_path = Arc::new(on_path);
    let drain_task = tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let drained = {
                let mut guard = pending.lock().await;
                std::mem::take(&mut *guard)
            };
            for path in drained {
                on_path(path);
            }
        }
    });

    Ok(DocsWatcher {
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
