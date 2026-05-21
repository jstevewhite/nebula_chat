//! Embedding provider abstraction. Two impls land in Phase 1 / Phase 4:
//! - `FastembedProvider` (local ONNX, behind the `local-embeddings` feature)
//! - `RemoteEmbeddingProvider` (Phase 4, uses configured LLM provider's API)
//!
//! The `dim()` reported by the active provider is recorded in `memory_meta` so
//! mismatches trigger a reindex at startup.

use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn model_id(&self) -> &str;
    fn dim(&self) -> usize;
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

#[cfg(feature = "local-embeddings")]
pub mod fastembed_provider;
pub mod remote;

#[cfg(feature = "local-embeddings")]
pub use fastembed_provider::FastembedProvider;
pub use remote::RemoteEmbeddingProvider;

/// Pack a slice of f32 into little-endian bytes for SQLite BLOB storage.
pub fn pack_f32(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Unpack a SQLite BLOB into a Vec<f32>. Returns an error if `bytes.len()`
/// isn't a multiple of 4.
pub fn unpack_f32(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return Err(anyhow::anyhow!(
            "embedding blob has length {} (not a multiple of 4)",
            bytes.len()
        ));
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        let arr: [u8; 4] = [chunk[0], chunk[1], chunk[2], chunk[3]];
        out.push(f32::from_le_bytes(arr));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        let v = vec![1.0f32, -2.5, 3.14, 0.0];
        let bytes = pack_f32(&v);
        let back = unpack_f32(&bytes).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn unpack_rejects_malformed_length() {
        assert!(unpack_f32(&[1, 2, 3]).is_err());
    }
}
