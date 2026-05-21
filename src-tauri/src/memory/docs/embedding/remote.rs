//! Remote embedding provider. Calls the configured LLM provider's embedding
//! endpoint. Two transports today:
//! - OpenAI / OpenAI-compatible: `POST {base}/embeddings` with `{model, input}`,
//!   reads `data[].embedding`.
//! - Ollama: `POST {base}/api/embed` with `{model, input}`, reads `embeddings`.
//!
//! Anthropic does not expose a public embeddings endpoint and is not supported
//! here; pick a different provider for `memory_remote_embedding_provider_id`.

use super::EmbeddingProvider;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::mcp::config::{ProviderConfig, ProviderType};

pub struct RemoteEmbeddingProvider {
    // memory_meta records `{provider_id}::{model_id}` so a provider switch also
    // triggers a re-embed pass, not just a model rename.
    composite_id: String,
    model_id: String,
    dim: usize,
    transport: Transport,
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

enum Transport {
    OpenAi,
    Ollama,
}

impl RemoteEmbeddingProvider {
    /// Probe the configured remote provider once at startup to learn its
    /// embedding dimension (we cannot ask: response shapes vary, so we embed a
    /// tiny canary string and measure).
    pub async fn try_new(
        provider_id: String,
        model_id: String,
        config: &ProviderConfig,
    ) -> Result<Self> {
        let transport = match config.provider_type {
            ProviderType::OpenAI | ProviderType::OpenAICompatible => Transport::OpenAi,
            ProviderType::Ollama => Transport::Ollama,
            ProviderType::Anthropic => {
                return Err(anyhow!(
                    "Anthropic does not expose an embeddings endpoint; pick an OpenAI- or Ollama-compatible provider"
                ));
            }
        };
        let base_url = config
            .base_url
            .clone()
            .unwrap_or_else(|| default_base_url(&config.provider_type));
        let api_key = config.api_key.clone();
        let composite_id = format!("{provider_id}::{model_id}");

        let probe_only = Self {
            composite_id: composite_id.clone(),
            model_id: model_id.clone(),
            dim: 0,
            transport,
            client: reqwest::Client::new(),
            base_url,
            api_key,
        };

        let probe = probe_only
            .embed_batch_raw(&["probe".to_string()])
            .await
            .context("probe embedding to determine dimension")?;
        let dim = probe
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("remote embedding probe returned no vectors"))?
            .len();
        if dim == 0 {
            return Err(anyhow!("remote embedding probe returned a 0-length vector"));
        }

        Ok(Self { dim, ..probe_only })
    }

    async fn embed_batch_raw(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        match self.transport {
            Transport::OpenAi => self.embed_openai(texts).await,
            Transport::Ollama => self.embed_ollama(texts).await,
        }
    }

    async fn embed_openai(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        #[derive(Serialize)]
        struct Req<'a> {
            model: &'a str,
            input: &'a [String],
        }
        #[derive(Deserialize)]
        struct Item {
            embedding: Vec<f32>,
        }
        #[derive(Deserialize)]
        struct Resp {
            data: Vec<Item>,
        }

        let url = format!("{}/embeddings", self.base_url.trim_end_matches('/'));
        let mut req = self.client.post(&url).json(&Req {
            model: &self.model_id,
            input: texts,
        });
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.context("send openai embeddings request")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "openai embeddings returned {}: {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }
        let body: Resp = resp.json().await.context("parse openai embeddings response")?;
        Ok(body.data.into_iter().map(|i| i.embedding).collect())
    }

    async fn embed_ollama(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        #[derive(Serialize)]
        struct Req<'a> {
            model: &'a str,
            input: &'a [String],
        }
        #[derive(Deserialize)]
        struct Resp {
            embeddings: Vec<Vec<f32>>,
        }

        let url = format!("{}/api/embed", self.base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .json(&Req {
                model: &self.model_id,
                input: texts,
            })
            .send()
            .await
            .context("send ollama embed request")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "ollama embed returned {}: {}",
                status,
                body.chars().take(500).collect::<String>()
            ));
        }
        let body: Resp = resp.json().await.context("parse ollama embed response")?;
        Ok(body.embeddings)
    }
}

fn default_base_url(provider_type: &ProviderType) -> String {
    match provider_type {
        ProviderType::OpenAI | ProviderType::OpenAICompatible => "https://api.openai.com/v1".into(),
        ProviderType::Ollama => "http://localhost:11434".into(),
        ProviderType::Anthropic => "https://api.anthropic.com/v1".into(),
    }
}

#[async_trait]
impl EmbeddingProvider for RemoteEmbeddingProvider {
    // Returns the composite "{provider_id}::{model_id}" so memory_meta detects
    // provider switches, not just model renames. The name `model_id` is the
    // trait-defined accessor; the underlying field is intentionally renamed.
    #[allow(clippy::misnamed_getters)]
    fn model_id(&self) -> &str {
        &self.composite_id
    }

    fn dim(&self) -> usize {
        self.dim
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.embed_batch_raw(texts).await
    }
}
