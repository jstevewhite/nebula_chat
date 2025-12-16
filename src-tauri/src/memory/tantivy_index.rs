use anyhow::Result;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, ReloadPolicy, TantivyDocument};
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub message_id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
    pub score: f32,
}

/// Indexing operations that can be queued
#[derive(Debug)]
enum IndexOperation {
    AddDocument {
        conversation_id: String,
        role: String,
        content: String,
        message_id: String,
        created_at: String,
    },
    DeleteByMessageId {
        message_id: String,
    },
    DeleteByConversationId {
        conversation_id: String,
    },
    ClearIndex,
    CommitAndReload,
}

/// Background worker for batch processing indexing operations
struct IndexBatchProcessor {
    index: Index,
    reader: tantivy::IndexReader,
    receiver: mpsc::UnboundedReceiver<IndexOperation>,
    batch_size: usize,
    max_batch_delay: Duration,
}

impl IndexBatchProcessor {
    fn new(
        index: Index,
        reader: tantivy::IndexReader,
        receiver: mpsc::UnboundedReceiver<IndexOperation>,
        batch_size: usize,
        max_batch_delay: Duration,
    ) -> Self {
        Self {
            index,
            reader,
            receiver,
            batch_size,
            max_batch_delay,
        }
    }

    async fn run(mut self) {
        let mut batch = Vec::with_capacity(self.batch_size);
        let mut last_commit_time = Instant::now();

        loop {
            // Try to receive an operation (non-blocking)
            match self.receiver.try_recv() {
                Ok(operation) => {
                    match operation {
                        IndexOperation::CommitAndReload => {
                            // Force immediate commit
                            if !batch.is_empty() {
                                if let Err(e) = self.process_batch(&batch).await {
                                    tracing::error!("Error processing batch: {}", e);
                                }
                                batch.clear();
                            }
                            // Reload reader to see latest changes
                            if let Err(e) = self.reader.reload() {
                                tracing::error!("Error reloading reader: {}", e);
                            }
                            last_commit_time = Instant::now();
                        }
                        op => {
                            batch.push(op);

                            // Check if we should commit due to batch size or time
                            let should_commit_by_size = batch.len() >= self.batch_size;
                            let should_commit_by_time = last_commit_time.elapsed() >= self.max_batch_delay;

                            if should_commit_by_size || should_commit_by_time {
                                if let Err(e) = self.process_batch(&batch).await {
                                    tracing::error!("Error processing batch: {}", e);
                                }
                                batch.clear();
                                last_commit_time = Instant::now();

                                // Reload reader to see latest changes
                                if let Err(e) = self.reader.reload() {
                                    tracing::error!("Error reloading reader: {}", e);
                                }
                            }
                        }
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    // No operations available, check time-based commit
                    if last_commit_time.elapsed() >= self.max_batch_delay && !batch.is_empty() {
                        if let Err(e) = self.process_batch(&batch).await {
                            tracing::error!("Error processing batch: {}", e);
                        }
                        batch.clear();
                        last_commit_time = Instant::now();

                        // Reload reader to see latest changes
                        if let Err(e) = self.reader.reload() {
                            tracing::error!("Error reloading reader: {}", e);
                        }
                    }
                    // Sleep briefly to avoid busy waiting
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    // Channel closed, process remaining batch and exit
                    break;
                }
            }
        }

        // Process any remaining operations before exiting
        if !batch.is_empty() {
            let _ = self.process_batch(&batch).await;
        }
    }

    async fn process_batch(&self, batch: &[IndexOperation]) -> Result<()> {
        let mut index_writer = self.index.writer::<TantivyDocument>(50_000_000)?;
        let schema = self.index.schema();
        let conv_id = schema.get_field("conversation_id").expect("schema");
        let role_field = schema.get_field("role").expect("schema");
        let content_field = schema.get_field("content").expect("schema");
        let msg_id_field = schema.get_field("message_id").expect("schema");
        let created_at_field = schema.get_field("created_at").expect("schema");

        for operation in batch {
            match operation {
                IndexOperation::AddDocument {
                    conversation_id,
                    role,
                    content,
                    message_id,
                    created_at,
                } => {
                    index_writer.add_document(doc!(
                        conv_id => conversation_id.as_str(),
                        role_field => role.as_str(),
                        content_field => content.as_str(),
                        msg_id_field => message_id.as_str(),
                        created_at_field => created_at.as_str()
                    ))?;
                }
                IndexOperation::DeleteByMessageId { message_id } => {
                    let term = tantivy::Term::from_field_text(msg_id_field, message_id);
                    index_writer.delete_term(term);
                }
                IndexOperation::DeleteByConversationId { conversation_id } => {
                    let term = tantivy::Term::from_field_text(conv_id, conversation_id);
                    index_writer.delete_term(term);
                }
                IndexOperation::ClearIndex => {
                    index_writer.delete_all_documents()?;
                }
                IndexOperation::CommitAndReload => {
                    // This is handled separately
                }
            }
        }

        index_writer.commit()?;
        Ok(())
    }
}

pub struct TantivyIndex {
    index: Index,
    reader: tantivy::IndexReader,
    sender: mpsc::UnboundedSender<IndexOperation>,
    processor_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl TantivyIndex {
    pub fn new(path: &str) -> Result<Self> {
        let index_path = std::path::Path::new(path);
        std::fs::create_dir_all(index_path)?;

        let mut schema_builder = Schema::builder();
        schema_builder.add_text_field("conversation_id", STRING | STORED);
        schema_builder.add_text_field("role", STRING | STORED);
        schema_builder.add_text_field("content", TEXT | STORED);
        schema_builder.add_text_field("message_id", STRING | STORED);
        schema_builder.add_text_field("created_at", STRING | STORED);
        let schema = schema_builder.build();

        let index = match Index::open_or_create(
            tantivy::directory::MmapDirectory::open(index_path)?,
            schema.clone(),
        ) {
            Ok(index) => index,
            Err(e) => {
                tracing::warn!(
                    "Failed to open index (likely schema mismatch). Recreating: {}",
                    e
                );
                // Remove all files in the directory
                for entry in std::fs::read_dir(index_path)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_file() {
                        std::fs::remove_file(path)?;
                    }
                }
                // Try again
                Index::create_in_dir(index_path, schema.clone())?
            }
        };

        // Reader
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;

        // Create channel for indexing operations
        let (sender, receiver) = mpsc::unbounded_channel::<IndexOperation>();

        // Start background processor
        let index_clone = index.clone();
        let reader_clone = reader.clone();

        let processor_handle = Arc::new(Mutex::new(Some(tokio::spawn(async move {
            IndexBatchProcessor::new(
                index_clone,
                reader_clone,
                receiver,
                10,      // Batch size: commit after 10 documents
                Duration::from_secs(2), // Max delay: commit after 2 seconds
            ).run().await
        }))));

        Ok(Self {
            index,
            reader,
            sender,
            processor_handle,
        })
    }

    pub fn add_document(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        message_id: &str,
        created_at: &str,
    ) -> Result<()> {
        // Queue the document for batch processing
        self.sender.send(IndexOperation::AddDocument {
            conversation_id: conversation_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            message_id: message_id.to_string(),
            created_at: created_at.to_string(),
        }).map_err(|e| anyhow::anyhow!("Failed to queue document: {}", e))?;

        Ok(())
    }

    pub fn search(&self, query_str: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let searcher = self.reader.searcher();
        let schema = self.index.schema();
        let content_field = schema.get_field("content").expect("schema");
        let conv_id_field = schema.get_field("conversation_id").expect("schema");
        let role_field = schema.get_field("role").expect("schema");
        let msg_id_field = schema.get_field("message_id").expect("schema");
        let created_at_field = schema.get_field("created_at").expect("schema");

        let query_parser = QueryParser::for_index(&self.index, vec![content_field]);
        let query = query_parser.parse_query(query_str)?;

        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

        let mut results = Vec::new();
        for (score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;

            let conversation_id = retrieved_doc
                .get_first(conv_id_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let role = retrieved_doc
                .get_first(role_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let content = retrieved_doc
                .get_first(content_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let message_id = retrieved_doc
                .get_first(msg_id_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let created_at = retrieved_doc
                .get_first(created_at_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            results.push(SearchResult {
                message_id,
                conversation_id,
                role,
                content,
                created_at,
                score,
            });
        }

        Ok(results)
    }

    pub fn delete_by_message_id(&self, message_id: &str) -> Result<()> {
        self.sender.send(IndexOperation::DeleteByMessageId {
            message_id: message_id.to_string(),
        }).map_err(|e| anyhow::anyhow!("Failed to queue deletion: {}", e))?;
        Ok(())
    }

    pub fn delete_by_conversation_id(&self, conversation_id: &str) -> Result<()> {
        self.sender.send(IndexOperation::DeleteByConversationId {
            conversation_id: conversation_id.to_string(),
        }).map_err(|e| anyhow::anyhow!("Failed to queue deletion: {}", e))?;
        Ok(())
    }

    pub fn clear_index(&self) -> Result<()> {
        self.sender.send(IndexOperation::ClearIndex).map_err(|e| anyhow::anyhow!("Failed to queue clear: {}", e))?;
        Ok(())
    }

    /// Force immediate commit and reload for cases where consistency is needed
    pub fn flush(&self) -> Result<()> {
        self.sender.send(IndexOperation::CommitAndReload).map_err(|e| anyhow::anyhow!("Failed to queue flush: {}", e))?;
        Ok(())
    }
}
