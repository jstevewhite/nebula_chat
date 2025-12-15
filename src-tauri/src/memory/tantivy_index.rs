use anyhow::Result;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, ReloadPolicy, TantivyDocument};

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub message_id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
    pub score: f32,
}

pub struct TantivyIndex {
    index: Index,
    reader: tantivy::IndexReader,
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

        Ok(Self { index, reader })
    }

    pub fn add_document(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        message_id: &str,
        created_at: &str,
    ) -> Result<()> {
        let mut index_writer = self.index.writer::<TantivyDocument>(50_000_000)?;

        let schema = self.index.schema();
        let conv_id = schema.get_field("conversation_id").expect("schema");
        let role_field = schema.get_field("role").expect("schema");
        let content_field = schema.get_field("content").expect("schema");
        let msg_id_field = schema.get_field("message_id").expect("schema");
        let created_at_field = schema.get_field("created_at").expect("schema");

        index_writer.add_document(doc!(
            conv_id => conversation_id,
            role_field => role,
            content_field => content,
            msg_id_field => message_id,
            created_at_field => created_at
        ))?;

        index_writer.commit()?;
        self.reader.reload()?;
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
        let mut index_writer = self.index.writer::<TantivyDocument>(50_000_000)?;
        let schema = self.index.schema();
        let msg_id_field = schema.get_field("message_id").expect("schema");
        let term = tantivy::Term::from_field_text(msg_id_field, message_id);
        index_writer.delete_term(term);
        index_writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub fn delete_by_conversation_id(&self, conversation_id: &str) -> Result<()> {
        let mut index_writer = self.index.writer::<TantivyDocument>(50_000_000)?;
        let schema = self.index.schema();
        let conv_id_field = schema.get_field("conversation_id").expect("schema");
        let term = tantivy::Term::from_field_text(conv_id_field, conversation_id);
        index_writer.delete_term(term);
        index_writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub fn clear_index(&self) -> Result<()> {
        let mut index_writer = self.index.writer::<TantivyDocument>(50_000_000)?;
        index_writer.delete_all_documents()?;
        index_writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }
}
