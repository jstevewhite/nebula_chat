//! Local embedding provider using `fastembed-rs` (ONNX runtime under the hood).
//!
//! Default model: `BAAI/bge-small-en-v1.5` (384 dim). The model is downloaded
//! to the cache dir on first use; subsequent runs load from disk.

use super::EmbeddingProvider;
use anyhow::{Context, Result};
use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::Arc;
use tokio::sync::Mutex;

const DEFAULT_MODEL_ID: &str = "bge-small-en-v1.5";
const DEFAULT_DIM: usize = 384;

pub struct FastembedProvider {
    model_id: String,
    dim: usize,
    model: Arc<Mutex<TextEmbedding>>,
}

impl FastembedProvider {
    /// Initialise the default BGE-small model. Blocking work happens in
    /// `spawn_blocking` so callers can await this from an async context
    /// without stalling the runtime.
    pub async fn try_default() -> Result<Self> {
        let model = tokio::task::spawn_blocking(|| {
            TextEmbedding::try_new(
                InitOptions::new(EmbeddingModel::BGESmallENV15)
                    .with_show_download_progress(true),
            )
        })
        .await
        .context("join fastembed init task")??;

        Ok(Self {
            model_id: DEFAULT_MODEL_ID.to_string(),
            dim: DEFAULT_DIM,
            model: Arc::new(Mutex::new(model)),
        })
    }
}

#[async_trait]
impl EmbeddingProvider for FastembedProvider {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dim(&self) -> usize {
        self.dim
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let texts = texts.to_vec();
        let model = self.model.clone();
        let embeddings = tokio::task::spawn_blocking(move || {
            let guard = model.blocking_lock();
            guard.embed(texts, None)
        })
        .await
        .context("join fastembed embed task")??;
        Ok(embeddings)
    }
}
