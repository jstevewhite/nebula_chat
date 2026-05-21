//! `DocStore`: facade for the docs subsystem. Coordinates markdown FS, SQLite
//! rows, the docs Tantivy index, the in-memory VectorCache, and the file
//! watcher. Exposes a small async API matching the six `memory_*` tools plus
//! a startup reconciliation pass.

pub mod api;
pub mod chunker;
pub mod embedding;
pub mod inject;
pub mod links;
pub mod retrieval;
pub mod store;
pub mod tools;
pub mod watcher;

use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use tokio::sync::Mutex;

use crate::memory::librarian::Librarian;

use self::api::*;
use self::chunker::Chunk;
use self::embedding::{pack_f32, unpack_f32, EmbeddingProvider};
use self::retrieval::{
    fuse, k_candidates, DocsTantivyIndex, SharedVectorCache, VectorCache, DEFAULT_SCORE_FLOOR,
};
use self::store::{path_for, ParsedDoc};
use self::watcher::DocsWatcher;

const DEFAULT_DIM: usize = 384;

pub type ProgressSink = Arc<dyn Fn(&str, usize, usize) + Send + Sync>;

pub struct DocStore {
    docs_dir: PathBuf,
    librarian: Arc<Mutex<Librarian>>,
    tantivy: Arc<DocsTantivyIndex>,
    vectors: SharedVectorCache,
    embedder: Option<Arc<dyn EmbeddingProvider>>,
    doc_locks: StdMutex<HashMap<String, Arc<Mutex<()>>>>,
    watcher: StdMutex<Option<DocsWatcher>>,
    progress_sink: StdMutex<Option<ProgressSink>>,
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ReconcileSummary {
    pub scanned: usize,
    pub ingested: usize,
    pub updated: usize,
    pub deleted: usize,
    /// Number of docs re-embedded because the configured embedding model
    /// changed (vectors table got truncated).
    pub reembedded: usize,
}

impl DocStore {
    /// Construct a new DocStore. Caller must invoke `startup_reconcile` and
    /// `start_watcher` after construction.
    pub async fn new(
        docs_dir: PathBuf,
        librarian: Arc<Mutex<Librarian>>,
        tantivy_dir: PathBuf,
        embedder: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&docs_dir)?;
        let tantivy = Arc::new(DocsTantivyIndex::open(&tantivy_dir)?);
        let dim = embedder.as_ref().map(|e| e.dim()).unwrap_or(DEFAULT_DIM);
        let vectors = Arc::new(RwLock::new(VectorCache::new(dim)));

        // Verify embedding model compatibility; reset vectors if it changed.
        {
            let lib = librarian.lock().await;
            let prev_model = lib.sqlite.meta_get("embedding_model")?;
            let prev_dim = lib.sqlite.meta_get("embedding_dim")?;
            if let Some(ref e) = embedder {
                let needs_reset = prev_model.as_deref() != Some(e.model_id())
                    || prev_dim.as_deref() != Some(&e.dim().to_string());
                if needs_reset {
                    if prev_model.is_some() || prev_dim.is_some() {
                        tracing::warn!(
                            "Embedding model changed (prev={:?}/{:?}, now={}/{}); truncating vectors",
                            prev_model,
                            prev_dim,
                            e.model_id(),
                            e.dim()
                        );
                        lib.sqlite.truncate_doc_chunk_vecs()?;
                        // The docs Tantivy index does not depend on dim; keep it.
                    }
                    lib.sqlite.meta_set("embedding_model", e.model_id())?;
                    lib.sqlite.meta_set("embedding_dim", &e.dim().to_string())?;
                }
            }
        }

        Ok(Arc::new(Self {
            docs_dir,
            librarian,
            tantivy,
            vectors,
            embedder,
            doc_locks: StdMutex::new(HashMap::new()),
            watcher: StdMutex::new(None),
            progress_sink: StdMutex::new(None),
        }))
    }

    pub fn docs_dir(&self) -> &Path {
        &self.docs_dir
    }

    /// Re-embed every doc on disk. Used when the embedding provider or model
    /// changes and the vectors table has been truncated. Returns the number
    /// of docs successfully re-ingested. Emits `memory:reembed-progress`
    /// events via the optional progress sink set with `set_progress_sink`.
    pub async fn reembed_all(self: &Arc<Self>) -> Result<usize> {
        let files = store::scan_dir(&self.docs_dir)?;
        let total = files.len();
        let mut done = 0;
        self.emit_progress("reembed", 0, total);
        for path in files {
            match store::read_path(&path) {
                Ok(parsed) => {
                    // Force re-ingest by skipping the mtime short-circuit.
                    let mut parsed = parsed;
                    parsed.mtime_ns = i64::MAX;
                    if let Err(e) = self.ingest_parsed(parsed).await {
                        tracing::warn!("reembed: failed on {}: {e}", path.display());
                    } else {
                        done += 1;
                    }
                }
                Err(e) => tracing::warn!("reembed: skip unreadable {}: {e}", path.display()),
            }
            self.emit_progress("reembed", done, total);
        }
        Ok(done)
    }

    /// Install a sink for progress events. Events look like
    /// `(phase, current, total)`. Used by the frontend to render a banner.
    pub fn set_progress_sink(&self, sink: ProgressSink) {
        *self.progress_sink.lock().expect("progress sink poisoned") = Some(sink);
    }

    fn emit_progress(&self, phase: &str, current: usize, total: usize) {
        if let Some(sink) = self
            .progress_sink
            .lock()
            .expect("progress sink poisoned")
            .as_ref()
        {
            sink(phase, current, total);
        }
    }

    pub fn embedder(&self) -> Option<&Arc<dyn EmbeddingProvider>> {
        self.embedder.as_ref()
    }

    async fn lock_doc(&self, id: &str) -> Arc<Mutex<()>> {
        let mut guard = self.doc_locks.lock().expect("doc_locks poisoned");
        guard
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Reconcile the on-disk docs with the SQLite + cache state before the
    /// watcher arms. Handles offline edits (`git pull`, manual `vim`, etc.).
    pub async fn startup_reconcile(self: &Arc<Self>) -> Result<ReconcileSummary> {
        let mut summary = ReconcileSummary::default();

        let on_disk = store::scan_dir(&self.docs_dir)?;
        summary.scanned = on_disk.len();

        // Build a map of (id -> (mtime_ns, path)) from SQLite.
        let db_docs: Vec<(String, String, String, String, i64, String, String)> = {
            let lib = self.librarian.lock().await;
            lib.sqlite.list_doc_rows()?
        };
        let mut db_map: HashMap<String, (i64, PathBuf)> = HashMap::new();
        for (id, _title, _tags, path, mtime, _c, _u) in db_docs {
            db_map.insert(id, (mtime, PathBuf::from(path)));
        }

        // Files on disk -> ingest if new or modified.
        let mut on_disk_ids = std::collections::HashSet::new();
        for path in on_disk {
            let parsed = match store::read_path(&path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("skip unreadable doc {}: {e}", path.display());
                    continue;
                }
            };
            let id = parsed.front.id.clone();
            if !links::is_valid_slug(&id) {
                tracing::warn!("skip doc with invalid id '{id}' at {}", path.display());
                continue;
            }
            on_disk_ids.insert(id.clone());

            match db_map.get(&id) {
                Some((mtime, _)) if *mtime >= parsed.mtime_ns => {
                    // Up to date; nothing to do.
                }
                Some(_) => {
                    self.ingest_parsed(parsed).await?;
                    summary.updated += 1;
                }
                None => {
                    self.ingest_parsed(parsed).await?;
                    summary.ingested += 1;
                }
            }
        }

        // DB rows whose files are gone -> delete.
        let to_delete: Vec<String> = db_map
            .keys()
            .filter(|id| !on_disk_ids.contains(*id))
            .cloned()
            .collect();
        for id in &to_delete {
            self.delete_internal(id).await?;
            summary.deleted += 1;
        }

        // Rebuild vector cache from SQLite.
        self.reload_vector_cache().await?;

        // Re-embed migration: if any docs exist on disk but the vector cache
        // is empty (e.g. the user switched embedding model and we truncated
        // doc_chunk_vecs), regenerate vectors for every chunk by re-ingesting
        // each doc. Embedding-free runs (no provider) are skipped.
        if self.embedder.is_some() && !on_disk_ids.is_empty() {
            let cache_empty = {
                let guard = self.vectors.read().expect("vector cache poisoned");
                guard.is_empty()
            };
            if cache_empty {
                tracing::info!(
                    "vector cache empty after reconcile; re-embedding {} docs",
                    on_disk_ids.len()
                );
                summary.reembedded = self.reembed_all().await?;
            }
        }

        Ok(summary)
    }

    /// Bulk-load the vector cache from SQLite. Called at startup and after
    /// any operation that could leave the cache stale.
    pub async fn reload_vector_cache(&self) -> Result<()> {
        let rows = {
            let lib = self.librarian.lock().await;
            lib.sqlite.load_all_chunk_vecs()?
        };
        let dim = self.embedder.as_ref().map(|e| e.dim()).unwrap_or(DEFAULT_DIM);
        let mut cache = VectorCache::new(dim);
        for (cid, did, bytes) in rows {
            match unpack_f32(&bytes) {
                Ok(v) if v.len() == dim => cache.upsert(cid, &did, v),
                Ok(v) => tracing::warn!(
                    "chunk {cid}: vector dim {} != expected {dim}; skipping",
                    v.len()
                ),
                Err(e) => tracing::warn!("chunk {cid}: bad embedding blob: {e}"),
            }
        }
        let mut guard = self.vectors.write().expect("vector cache poisoned");
        *guard = cache;
        Ok(())
    }

    /// Start the FS watcher. Calls into `ingest_path` for every changed file.
    pub fn start_watcher(self: &Arc<Self>) -> Result<()> {
        let me = self.clone();
        let handle = watcher::start_watching(self.docs_dir.clone(), move |path| {
            let me = me.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = me.ingest_or_delete_path(&path).await {
                    tracing::warn!("watcher ingest failed for {}: {e}", path.display());
                }
            });
        })?;
        *self.watcher.lock().expect("watcher slot poisoned") = Some(handle);
        Ok(())
    }

    async fn ingest_or_delete_path(self: &Arc<Self>, path: &Path) -> Result<()> {
        if !path.exists() {
            // File removed. Derive the id from the filename and forget.
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if links::is_valid_slug(stem) {
                    self.delete_internal(stem).await?;
                }
            }
            return Ok(());
        }
        let parsed = store::read_path(path)?;
        self.ingest_parsed(parsed).await?;
        Ok(())
    }

    /// Ingest a parsed doc: write SQLite rows + Tantivy + cache. Honours the
    /// per-doc lock and skips no-op writes when mtime_ns matches the DB.
    pub async fn ingest_parsed(&self, parsed: ParsedDoc) -> Result<()> {
        let id = parsed.front.id.clone();
        let lock = self.lock_doc(&id).await;
        let _guard = lock.lock().await;

        // Skip if SQLite already has this mtime or newer (idempotent reingest).
        {
            let lib = self.librarian.lock().await;
            if let Some((_t, _tags, _p, prev_mtime, _c, _u)) = lib.sqlite.get_doc_row(&id)? {
                if prev_mtime >= parsed.mtime_ns {
                    tracing::debug!("ingest: {} up to date (mtime)", id);
                    return Ok(());
                }
            }
        }

        let body = parsed.body.clone();
        let merged_links = links::merge_links(&parsed.front.links, &body);
        let chunks = chunker::chunk(&body);

        // Embed chunks if a provider is configured.
        let embeddings: Vec<Vec<f32>> = if let Some(emb) = &self.embedder {
            let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
            emb.embed_batch(&texts).await.unwrap_or_else(|e| {
                tracing::warn!("embed_batch failed for {}: {e}", id);
                Vec::new()
            })
        } else {
            Vec::new()
        };

        let inserted: Vec<(i64, String, usize)> = {
            let lib = self.librarian.lock().await;

            // Existing created_at preserved if row exists.
            let prev_created = lib
                .sqlite
                .get_doc_row(&id)?
                .map(|(_t, _tags, _p, _m, c, _u)| c)
                .unwrap_or_else(|| parsed.front.created_at.clone());

            let tags_json = serde_json::to_string(&parsed.front.tags)?;
            let updated_at = if parsed.front.updated_at.is_empty() {
                chrono::Utc::now().to_rfc3339()
            } else {
                parsed.front.updated_at.clone()
            };
            let created_at = if prev_created.is_empty() {
                updated_at.clone()
            } else {
                prev_created
            };

            lib.sqlite.upsert_doc_row(
                &id,
                &parsed.front.title,
                &tags_json,
                parsed
                    .path
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("non-utf8 path"))?,
                parsed.mtime_ns,
                &created_at,
                &updated_at,
            )?;

            let chunk_payload: Vec<(String, String, usize, usize)> = chunks
                .iter()
                .map(|c: &Chunk| {
                    (
                        c.text.clone(),
                        c.text_hash.clone(),
                        c.char_start,
                        c.char_end,
                    )
                })
                .collect();
            let inserted = lib.sqlite.replace_doc_chunks(&id, &chunk_payload)?;

            // Embeddings → vectors table.
            if !embeddings.is_empty() && embeddings.len() == inserted.len() {
                for ((cid, _h, _o), emb) in inserted.iter().zip(embeddings.iter()) {
                    let bytes = pack_f32(emb);
                    lib.sqlite.insert_doc_chunk_vec(*cid, &bytes)?;
                }
            }

            lib.sqlite.replace_doc_links(&id, &merged_links)?;

            inserted
        };

        // Update Tantivy.
        let tantivy_chunks: Vec<(i64, i64, String)> = inserted
            .iter()
            .zip(chunks.iter())
            .map(|((cid, _h, ord), c)| (*cid, *ord as i64, c.text.clone()))
            .collect();
        self.tantivy.replace_doc_chunks(&id, &tantivy_chunks)?;

        // Update in-memory vector cache.
        {
            retrieval::purge_doc_from_cache(&self.vectors, &id);
            if !embeddings.is_empty() && embeddings.len() == inserted.len() {
                let entries = inserted
                    .iter()
                    .zip(embeddings.into_iter())
                    .map(|((cid, _h, _ord), emb)| (*cid, id.clone(), emb));
                retrieval::load_cache_entries(&self.vectors, entries);
            }
        }

        Ok(())
    }

    async fn delete_internal(&self, id: &str) -> Result<()> {
        let lock = self.lock_doc(id).await;
        let _guard = lock.lock().await;

        {
            let lib = self.librarian.lock().await;
            lib.sqlite.delete_doc_row(id)?;
        }
        self.tantivy.delete_doc(id)?;
        retrieval::purge_doc_from_cache(&self.vectors, id);
        let _ = store::delete_doc(&self.docs_dir, id);
        Ok(())
    }

    // ---------- Tool-facing API ----------

    pub async fn remember(&self, input: RememberInput) -> std::result::Result<RememberOutput, DocsError> {
        if !links::is_valid_slug(&input.id) {
            return Err(DocsError::new(
                "INVALID_ID",
                "id must match [a-z0-9][a-z0-9-]{0,63}",
            ));
        }
        // Reject if a row already exists.
        {
            let lib = self.librarian.lock().await;
            let exists = lib
                .sqlite
                .get_doc_row(&input.id)
                .map_err(|e| DocsError::new("INTERNAL", e.to_string()))?
                .is_some();
            if exists {
                return Err(DocsError::new(
                    "ALREADY_EXISTS",
                    format!("doc '{}' already exists; use memory_edit", input.id),
                ));
            }
        }
        if path_for(&self.docs_dir, &input.id).exists() {
            return Err(DocsError::new(
                "ALREADY_EXISTS",
                format!("file for '{}' already on disk", input.id),
            ));
        }

        let (path, _c, updated_at) = store::write_doc(
            &self.docs_dir,
            &input.id,
            &input.title,
            &input.tags,
            &input.links,
            &input.content,
            None,
        )
        .map_err(|e| DocsError::new("INTERNAL", e.to_string()))?;

        let parsed = store::read_path(&path)
            .map_err(|e| DocsError::new("INTERNAL", e.to_string()))?;
        self.ingest_parsed(parsed)
            .await
            .map_err(|e| DocsError::new("INTERNAL", e.to_string()))?;

        Ok(RememberOutput {
            id: input.id,
            path: path.to_string_lossy().to_string(),
            updated_at,
        })
    }

    pub async fn fetch(&self, id: &str) -> Result<Option<DocRecord>> {
        let lib = self.librarian.lock().await;
        let row = lib.sqlite.get_doc_row(id)?;
        let Some((title, tags_json, _path, _mtime, created_at, updated_at)) = row else {
            return Ok(None);
        };
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        let outbound = lib.sqlite.doc_outbound_links(id)?;
        drop(lib);

        let parsed = store::read_id(&self.docs_dir, id)?;
        let content = parsed
            .map(|p| p.body)
            .unwrap_or_default();

        Ok(Some(DocRecord {
            id: id.to_string(),
            title,
            tags,
            links: outbound,
            content,
            created_at,
            updated_at,
        }))
    }

    pub async fn edit(&self, input: EditInput) -> std::result::Result<EditOutput, DocsError> {
        if input.replace.is_some() == input.append.is_some() {
            return Err(DocsError::new(
                "BAD_REQUEST",
                "exactly one of `replace` or `append` must be provided",
            ));
        }

        let lock = self.lock_doc(&input.id).await;
        let _guard = lock.lock().await;

        let parsed = match store::read_id(&self.docs_dir, &input.id)
            .map_err(|e| DocsError::new("INTERNAL", e.to_string()))?
        {
            Some(p) => p,
            None => {
                return Err(DocsError::new(
                    "NOT_FOUND",
                    format!("doc '{}' does not exist", input.id),
                ))
            }
        };

        // Optimistic concurrency check.
        if let Some(expected) = &input.expected_updated_at {
            if expected != &parsed.front.updated_at {
                return Err(DocsError::conflict(parsed.front.updated_at.clone()));
            }
        }

        // Compute new body.
        let new_body = if let Some(rep) = &input.replace {
            let occurrences = parsed.body.matches(&rep.find).count();
            if occurrences == 0 {
                return Err(DocsError::new(
                    "FIND_NOT_FOUND",
                    "`replace.find` did not match the body",
                ));
            }
            if occurrences > 1 {
                return Err(DocsError::new(
                    "FIND_AMBIGUOUS",
                    format!("`replace.find` matched {} times; must be unique", occurrences),
                ));
            }
            parsed.body.replacen(&rep.find, &rep.with, 1)
        } else if let Some(text) = &input.append {
            let mut out = parsed.body.clone();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(text);
            out
        } else {
            unreachable!("guarded above")
        };

        let (path, _c, updated_at) = store::write_doc(
            &self.docs_dir,
            &input.id,
            &parsed.front.title,
            &parsed.front.tags,
            &parsed.front.links,
            &new_body,
            Some(&parsed.front.created_at),
        )
        .map_err(|e| DocsError::new("INTERNAL", e.to_string()))?;

        let reread = store::read_path(&path)
            .map_err(|e| DocsError::new("INTERNAL", e.to_string()))?;
        self.ingest_parsed(reread)
            .await
            .map_err(|e| DocsError::new("INTERNAL", e.to_string()))?;

        Ok(EditOutput {
            id: input.id,
            updated_at,
            new_length: new_body.len(),
        })
    }

    pub async fn forget(&self, id: &str) -> std::result::Result<(), DocsError> {
        if !links::is_valid_slug(id) {
            return Err(DocsError::new("INVALID_ID", "id is not a valid slug"));
        }
        self.delete_internal(id)
            .await
            .map_err(|e| DocsError::new("INTERNAL", e.to_string()))?;
        Ok(())
    }

    pub async fn recall(&self, input: RecallInput) -> Result<RecallOutput> {
        let k = input.k.clamp(1, 10);
        let k_cand = k_candidates(k);

        // Embedding cosine.
        let cosine_hits: Vec<(i64, String, f32)> = if let Some(emb) = &self.embedder {
            let q_emb = emb
                .embed_batch(std::slice::from_ref(&input.query))
                .await?;
            if let Some(q) = q_emb.into_iter().next() {
                let cache = self.vectors.read().expect("vector cache poisoned");
                cache.cosine_top_k(&q, k_cand)
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let bm25_hits = self.tantivy.search(&input.query, k_cand).unwrap_or_default();

        // Resolve chunk text once per candidate.
        let mut needed: Vec<i64> = cosine_hits
            .iter()
            .map(|t| t.0)
            .chain(bm25_hits.iter().map(|t| t.0))
            .collect();
        needed.sort_unstable();
        needed.dedup();

        let mut text_map: HashMap<i64, (String, i64, String)> = HashMap::new();
        {
            let lib = self.librarian.lock().await;
            for cid in &needed {
                if let Some((did, ord, text, _h)) = lib.sqlite.get_doc_chunk(*cid)? {
                    text_map.insert(*cid, (did, ord, text));
                }
            }
        }

        let hits = fuse(
            &cosine_hits,
            &bm25_hits,
            k,
            DEFAULT_SCORE_FLOOR,
            &|cid| text_map.get(&cid).cloned(),
        );

        Ok(RecallOutput { hits })
    }

    pub async fn link_context(&self, input: LinkContextInput) -> Result<LinkContextOutput> {
        let depth = input.depth.clamp(1, 3);
        let max_docs = input.max_docs.clamp(1, 20);

        let nodes_raw: Vec<(String, i64)> = {
            let lib = self.librarian.lock().await;
            lib.sqlite.link_bfs(&input.id, depth, max_docs)?
        };

        // Resolve titles + missing-ness for each node.
        let mut nodes = Vec::with_capacity(nodes_raw.len());
        let mut edges = Vec::new();
        {
            let lib = self.librarian.lock().await;
            for (id, d) in &nodes_raw {
                let row = lib.sqlite.get_doc_row(id)?;
                nodes.push(LinkNode {
                    id: id.clone(),
                    title: row.as_ref().map(|(t, _, _, _, _, _)| t.clone()),
                    depth: *d,
                    missing: row.is_none(),
                });
            }
            // Collect edges over the reachable set.
            let ids: std::collections::HashSet<String> = nodes_raw.iter().map(|t| t.0.clone()).collect();
            for id in &ids {
                let outbound = lib.sqlite.doc_outbound_links(id)?;
                for dst in outbound {
                    if ids.contains(&dst) {
                        edges.push(LinkEdge {
                            src: id.clone(),
                            dst,
                        });
                    }
                }
            }
        }

        Ok(LinkContextOutput {
            start: input.id,
            nodes,
            edges,
        })
    }

    /// Convenience: a UI-friendly list of all known docs.
    pub async fn list_docs(&self) -> Result<Vec<DocSummary>> {
        let lib = self.librarian.lock().await;
        let rows = lib.sqlite.list_doc_rows()?;
        let mut out = Vec::with_capacity(rows.len());
        for (id, title, tags_json, _path, _mtime, _c, updated_at) in rows {
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            out.push(DocSummary {
                id,
                title,
                tags,
                updated_at,
            });
        }
        Ok(out)
    }
}

impl std::fmt::Debug for DocStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocStore")
            .field("docs_dir", &self.docs_dir)
            .field("embedder", &self.embedder.as_ref().map(|e| e.model_id()))
            .field("vector_cache_len", &self.vectors.read().ok().map(|v| v.len()))
            .finish()
    }
}
