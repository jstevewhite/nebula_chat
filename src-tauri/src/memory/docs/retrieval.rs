//! Recall pipeline: embed query → cosine top-K against in-memory `VectorCache`
//! + BM25 top-K against the docs Tantivy index → fuse + diversify → top-K.

use anyhow::{Context, Result};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, RwLock};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, ReloadPolicy, TantivyDocument};

use super::api::{RecallHit, ScoreComponents};

const MAX_K_CANDIDATES: usize = 30;
const COSINE_WEIGHT: f32 = 0.6;
const BM25_WEIGHT: f32 = 0.4;
const PER_DOC_CAP: usize = 2;
pub const DEFAULT_SCORE_FLOOR: f32 = 0.15;

/// In-memory cache of all chunk embeddings. The cache is the authoritative
/// source for vector recall at runtime; the SQLite `doc_chunk_vecs` table is
/// the durability layer. All writes go through both.
#[derive(Default)]
pub struct VectorCache {
    entries: Vec<CacheEntry>,
    by_chunk: HashMap<i64, usize>, // chunk_id -> index in entries
    dim: usize,
}

struct CacheEntry {
    chunk_id: i64,
    doc_id: String,
    embedding: Vec<f32>,
    norm: f32, // pre-computed L2 norm
}

impl VectorCache {
    pub fn new(dim: usize) -> Self {
        Self {
            entries: Vec::new(),
            by_chunk: HashMap::new(),
            dim,
        }
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.by_chunk.clear();
    }

    pub fn upsert(&mut self, chunk_id: i64, doc_id: &str, embedding: Vec<f32>) {
        if embedding.len() != self.dim {
            tracing::warn!(
                "VectorCache::upsert: dim mismatch (got {}, expected {}); rejecting",
                embedding.len(),
                self.dim
            );
            return;
        }
        let norm = embedding.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
        let entry = CacheEntry {
            chunk_id,
            doc_id: doc_id.to_string(),
            embedding,
            norm,
        };
        match self.by_chunk.get(&chunk_id).copied() {
            Some(idx) => {
                self.entries[idx] = entry;
            }
            None => {
                let idx = self.entries.len();
                self.entries.push(entry);
                self.by_chunk.insert(chunk_id, idx);
            }
        }
    }

    pub fn remove_doc(&mut self, doc_id: &str) {
        let keep: Vec<CacheEntry> = std::mem::take(&mut self.entries)
            .into_iter()
            .filter(|e| e.doc_id != doc_id)
            .collect();
        self.entries = keep;
        self.by_chunk.clear();
        for (i, e) in self.entries.iter().enumerate() {
            self.by_chunk.insert(e.chunk_id, i);
        }
    }

    /// Brute-force cosine top-K via rayon. Returns (chunk_id, doc_id, cosine).
    pub fn cosine_top_k(&self, query: &[f32], k: usize) -> Vec<(i64, String, f32)> {
        if self.entries.is_empty() || query.len() != self.dim {
            return Vec::new();
        }
        let q_norm = query.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);

        let mut scored: Vec<(i64, String, f32)> = self
            .entries
            .par_iter()
            .map(|e| {
                let dot: f32 = e
                    .embedding
                    .iter()
                    .zip(query.iter())
                    .map(|(a, b)| a * b)
                    .sum();
                let cos = dot / (e.norm * q_norm);
                (e.chunk_id, e.doc_id.clone(), cos)
            })
            .collect();
        scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored
    }
}

/// Tantivy index dedicated to doc chunks. Lives at `<docs_index>/` next to
/// the message index. Separate index so we never touch the message schema
/// and can rebuild this one freely if the chunking strategy changes.
pub struct DocsTantivyIndex {
    index: Index,
    reader: tantivy::IndexReader,
    schema: Schema,
}

impl DocsTantivyIndex {
    pub fn open(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path)?;
        let mut sb = Schema::builder();
        sb.add_text_field("doc_id", STRING | STORED);
        sb.add_i64_field("chunk_id", STORED | INDEXED);
        sb.add_i64_field("ord", STORED);
        sb.add_text_field("content", TEXT | STORED);
        let schema = sb.build();

        let index = match Index::open_or_create(
            tantivy::directory::MmapDirectory::open(path)?,
            schema.clone(),
        ) {
            Ok(idx) => idx,
            Err(e) => {
                tracing::warn!(
                    "DocsTantivyIndex: failed to open ({e}); recreating in {}",
                    path.display()
                );
                for entry in std::fs::read_dir(path)? {
                    let p = entry?.path();
                    if p.is_file() {
                        let _ = std::fs::remove_file(p);
                    }
                }
                Index::create_in_dir(path, schema.clone())?
            }
        };

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;

        Ok(Self {
            index,
            reader,
            schema,
        })
    }

    fn field(&self, name: &str) -> Field {
        self.schema.get_field(name).expect("schema")
    }

    /// Delete every chunk for a doc, then index the supplied chunks.
    /// Single committed transaction.
    pub fn replace_doc_chunks(
        &self,
        doc_id: &str,
        chunks: &[(i64, i64, String)], // (chunk_id, ord, content)
    ) -> Result<()> {
        let mut writer = self.index.writer::<TantivyDocument>(50_000_000)?;
        let doc_id_f = self.field("doc_id");
        let chunk_id_f = self.field("chunk_id");
        let ord_f = self.field("ord");
        let content_f = self.field("content");

        writer.delete_term(Term::from_field_text(doc_id_f, doc_id));
        for (cid, ord, content) in chunks {
            writer.add_document(doc!(
                doc_id_f => doc_id.to_string(),
                chunk_id_f => *cid,
                ord_f => *ord,
                content_f => content.clone(),
            ))?;
        }
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub fn delete_doc(&self, doc_id: &str) -> Result<()> {
        let mut writer = self.index.writer::<TantivyDocument>(50_000_000)?;
        let doc_id_f = self.field("doc_id");
        writer.delete_term(Term::from_field_text(doc_id_f, doc_id));
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// BM25 search; returns (chunk_id, doc_id, bm25_score).
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<(i64, String, f32)>> {
        let searcher = self.reader.searcher();
        let content_f = self.field("content");
        let doc_id_f = self.field("doc_id");
        let chunk_id_f = self.field("chunk_id");

        let parser = QueryParser::for_index(&self.index, vec![content_f]);
        let q = match parser.parse_query(query) {
            Ok(q) => q,
            Err(_) => return Ok(Vec::new()),
        };
        let top = searcher.search(&q, &TopDocs::with_limit(limit))?;

        let mut out = Vec::with_capacity(top.len());
        for (score, addr) in top {
            let d: TantivyDocument = searcher.doc(addr).context("retrieve doc")?;
            let doc_id = d
                .get_first(doc_id_f)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let chunk_id = d
                .get_first(chunk_id_f)
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            out.push((chunk_id, doc_id, score));
        }
        Ok(out)
    }

    pub fn clear(&self) -> Result<()> {
        let mut writer = self.index.writer::<TantivyDocument>(50_000_000)?;
        writer.delete_all_documents()?;
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }
}

/// Fuse cosine and BM25 candidate lists into a final ranked top-K.
///
/// - Normalise each list to [0,1] (min-max within the list).
/// - Blend: `score = 0.6 * cosine + 0.4 * bm25`. Missing component contributes 0.
/// - Drop entries below `score_floor`.
/// - Cap to `PER_DOC_CAP` chunks per doc for diversification.
/// - Truncate to `k`.
pub fn fuse(
    cosine: &[(i64, String, f32)],
    bm25: &[(i64, String, f32)],
    k: usize,
    score_floor: f32,
    chunk_text_lookup: &impl Fn(i64) -> Option<(String, i64, String)>, // chunk_id -> (doc_id, ord, text)
) -> Vec<RecallHit> {
    let cosine_norm = normalise(cosine);
    let bm25_norm = normalise(bm25);

    let mut by_chunk: HashMap<i64, (f32, f32, String)> = HashMap::new();
    for (cid, did, n) in &cosine_norm {
        by_chunk.insert(*cid, (*n, 0.0, did.clone()));
    }
    for (cid, did, n) in &bm25_norm {
        by_chunk
            .entry(*cid)
            .and_modify(|(_, b, _)| *b = *n)
            .or_insert((0.0, *n, did.clone()));
    }

    let mut ranked: Vec<(i64, String, f32, f32, f32)> = by_chunk
        .into_iter()
        .map(|(cid, (cos, bm, did))| {
            let blended = COSINE_WEIGHT * cos + BM25_WEIGHT * bm;
            (cid, did, blended, cos, bm)
        })
        .filter(|t| t.2 >= score_floor)
        .collect();
    ranked.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut per_doc: HashMap<String, usize> = HashMap::new();
    let mut hits: Vec<RecallHit> = Vec::with_capacity(k);
    for (cid, did, score, cos, bm) in ranked {
        let cnt = per_doc.entry(did.clone()).or_insert(0);
        if *cnt >= PER_DOC_CAP {
            continue;
        }
        *cnt += 1;
        let (text, ord) = match chunk_text_lookup(cid) {
            Some((_, ord, t)) => (t, ord),
            None => continue,
        };
        hits.push(RecallHit {
            doc_id: did,
            chunk_id: cid,
            ord,
            text,
            score,
            score_components: ScoreComponents {
                cosine: cos,
                bm25: bm,
            },
        });
        if hits.len() >= k {
            break;
        }
    }
    hits
}

fn normalise(list: &[(i64, String, f32)]) -> Vec<(i64, String, f32)> {
    if list.is_empty() {
        return Vec::new();
    }
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for (_, _, s) in list {
        if *s < min {
            min = *s;
        }
        if *s > max {
            max = *s;
        }
    }
    let span = (max - min).abs().max(1e-9);
    list.iter()
        .map(|(cid, did, s)| (*cid, did.clone(), ((s - min) / span).clamp(0.0, 1.0)))
        .collect()
}

pub fn k_candidates(k: usize) -> usize {
    (k * 5).min(MAX_K_CANDIDATES).max(k)
}

/// Convenience type alias used elsewhere.
pub type SharedVectorCache = Arc<RwLock<VectorCache>>;

/// Discard a stale set of chunks for a doc from the cache. Used when chunk
/// IDs are reassigned (e.g. after `replace_doc_chunks` in SQLite).
pub fn purge_doc_from_cache(cache: &SharedVectorCache, doc_id: &str) {
    let mut guard = cache.write().expect("vector cache poisoned");
    guard.remove_doc(doc_id);
}

/// Subset of `Vec` operations used by callers to bulk-load the cache from
/// SQLite at startup.
pub fn load_cache_entries(
    cache: &SharedVectorCache,
    entries: impl IntoIterator<Item = (i64, String, Vec<f32>)>,
) {
    let mut guard = cache.write().expect("vector cache poisoned");
    for (cid, did, emb) in entries {
        guard.upsert(cid, &did, emb);
    }
}

/// Resolve unique doc IDs present in a candidate list. Used by callers that
/// want to expand chunk hits into doc-level scores.
pub fn unique_doc_ids(list: &[(i64, String, f32)]) -> HashSet<String> {
    list.iter().map(|(_, d, _)| d.clone()).collect()
}
