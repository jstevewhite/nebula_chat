use anyhow::Result;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, ReloadPolicy, TantivyDocument};

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
        let schema = schema_builder.build();

        let index = Index::open_or_create(
            tantivy::directory::MmapDirectory::open(index_path)?,
            schema.clone(),
        )?;

        // Reader
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;

        Ok(Self { index, reader })
    }

    pub fn add_document(&self, conversation_id: &str, role: &str, content: &str) -> Result<()> {
        let mut index_writer = self.index.writer(50_000_000)?;

        let schema = self.index.schema();
        let conv_id = schema.get_field("conversation_id").expect("schema");
        let role_field = schema.get_field("role").expect("schema");
        let content_field = schema.get_field("content").expect("schema");

        index_writer.add_document(doc!(
            conv_id => conversation_id,
            role_field => role,
            content_field => content
        ))?;

        index_writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub fn search(&self, query_str: &str, limit: usize) -> Result<Vec<(String, String)>> {
        let searcher = self.reader.searcher();
        let schema = self.index.schema();
        let content_field = schema.get_field("content").expect("schema");
        let conv_id_field = schema.get_field("conversation_id").expect("schema");

        let query_parser = QueryParser::for_index(&self.index, vec![content_field]);
        let query = query_parser.parse_query(query_str)?;

        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

        let mut results = Vec::new();
        for (_score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;
            let content = retrieved_doc
                .get_first(content_field)
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let id = retrieved_doc
                .get_first(conv_id_field)
                .and_then(|v| v.as_str())
                .unwrap_or("");
            results.push((id.to_string(), content.to_string()));
        }

        Ok(results)
    }
}
