use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TantivyDocument, Value as TantivyValue, FAST, INDEXED,
    STORED, TEXT,
};
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy, Term};

use crate::error::{HostError, HostResult};
use crate::storage::LocalSearchHit;

/// Genuine local full-text search engine backed by persistent Tantivy 0.22 (BM25 + mmap).
/// `reader` is lock-free (`Sync`) outside the writer mutex so concurrent searches never block behind commits.
pub struct HostSearchEngine {
    index_dir: PathBuf,
    index: Index,
    writer: Mutex<IndexWriter>,
    reader: IndexReader,
    f_message_id: Field,
    f_chat_id: Field,
    f_sender_id: Field,
    f_timestamp_ms: Field,
    f_body: Field,
}

impl HostSearchEngine {
    pub fn open(index_dir: &Path) -> HostResult<Self> {
        std::fs::create_dir_all(index_dir).map_err(|e| {
            HostError::storage(format!(
                "Failed to create Tantivy index directory {:?}: {e}",
                index_dir
            ))
        })?;

        let mut schema_builder = Schema::builder();
        let f_message_id = schema_builder.add_i64_field("message_id", INDEXED | STORED | FAST);
        let f_chat_id = schema_builder.add_i64_field("chat_id", INDEXED | STORED | FAST);
        let f_sender_id = schema_builder.add_u64_field("sender_id", STORED);
        let f_timestamp_ms = schema_builder.add_i64_field("timestamp_ms", STORED | FAST);
        let f_body = schema_builder.add_text_field("body", TEXT | STORED);
        let schema = schema_builder.build();

        let mmap_dir = MmapDirectory::open(index_dir)
            .map_err(|e| HostError::storage(format!("Failed to open Tantivy MmapDirectory: {e}")))?;
        let index = Index::open_or_create(mmap_dir, schema)
            .map_err(|e| HostError::storage(format!("Failed to open/create Tantivy index: {e}")))?;

        let loaded_schema = index.schema();
        let f_message_id = loaded_schema.get_field("message_id").unwrap_or(f_message_id);
        let f_chat_id = loaded_schema.get_field("chat_id").unwrap_or(f_chat_id);
        let f_sender_id = loaded_schema.get_field("sender_id").unwrap_or(f_sender_id);
        let f_timestamp_ms = loaded_schema
            .get_field("timestamp_ms")
            .unwrap_or(f_timestamp_ms);
        let f_body = loaded_schema.get_field("body").unwrap_or(f_body);

        // Single-threaded 16 MiB writer budget avoids thread/memory bloat on iOS and CI runners.
        let writer = index
            .writer_with_num_threads(1, 16_000_000)
            .map_err(|e| HostError::storage(format!("Failed to create Tantivy IndexWriter: {e}")))?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|e| HostError::storage(format!("Failed to create Tantivy IndexReader: {e}")))?;

        Ok(Self {
            index_dir: index_dir.to_path_buf(),
            index,
            writer: Mutex::new(writer),
            reader,
            f_message_id,
            f_chat_id,
            f_sender_id,
            f_timestamp_ms,
            f_body,
        })
    }

    pub fn index_dir(&self) -> &Path {
        &self.index_dir
    }

    pub fn index_message(
        &self,
        message_id: i64,
        chat_id: i64,
        sender_id: u64,
        timestamp_ms: i64,
        body: &str,
    ) -> HostResult<()> {
        let mut writer = self.writer.lock();
        let term = Term::from_field_i64(self.f_message_id, message_id);
        writer.delete_term(term);
        if let Err(e) = writer.add_document(doc!(
            self.f_message_id => message_id,
            self.f_chat_id => chat_id,
            self.f_sender_id => sender_id,
            self.f_timestamp_ms => timestamp_ms,
            self.f_body => body,
        )) {
            let _ = writer.rollback();
            return Err(HostError::storage(format!("Tantivy add_document failed: {e}")));
        }
        if let Err(e) = writer.commit() {
            let _ = writer.rollback();
            return Err(HostError::storage(format!("Tantivy commit failed: {e}")));
        }
        drop(writer);
        self.reader
            .reload()
            .map_err(|e| HostError::storage(format!("Tantivy reader reload failed: {e}")))?;
        Ok(())
    }

    pub fn rebuild(&self, messages: &[(i64, i64, u64, i64, String)]) -> HostResult<usize> {
        let mut writer = self.writer.lock();
        if let Err(e) = writer.delete_all_documents() {
            let _ = writer.rollback();
            return Err(HostError::storage(format!(
                "Tantivy delete_all_documents failed: {e}"
            )));
        }
        for (message_id, chat_id, sender_id, timestamp_ms, body) in messages {
            if let Err(e) = writer.add_document(doc!(
                self.f_message_id => *message_id,
                self.f_chat_id => *chat_id,
                self.f_sender_id => *sender_id,
                self.f_timestamp_ms => *timestamp_ms,
                self.f_body => body.as_str(),
            )) {
                let _ = writer.rollback();
                return Err(HostError::storage(format!("Tantivy add_document failed: {e}")));
            }
        }
        if let Err(e) = writer.commit() {
            let _ = writer.rollback();
            return Err(HostError::storage(format!("Tantivy rebuild commit failed: {e}")));
        }
        drop(writer);
        self.reader
            .reload()
            .map_err(|e| HostError::storage(format!("Tantivy rebuild reload failed: {e}")))?;
        Ok(messages.len())
    }

    pub fn doc_count(&self) -> u64 {
        self.reader.searcher().num_docs()
    }

    pub fn search(
        &self,
        query_str: &str,
        chat_id: Option<i64>,
        limit: usize,
    ) -> HostResult<Vec<LocalSearchHit>> {
        let trimmed = query_str.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 200);
        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.f_body]);
        let (parsed_query, _parse_errors) = query_parser.parse_query_lenient(trimmed);

        let final_query: Box<dyn Query> = if let Some(cid) = chat_id {
            let chat_term = Term::from_field_i64(self.f_chat_id, cid);
            let chat_query: Box<dyn Query> =
                Box::new(TermQuery::new(chat_term, IndexRecordOption::Basic));
            Box::new(BooleanQuery::new(vec![
                (Occur::Must, parsed_query),
                (Occur::Must, chat_query),
            ]))
        } else {
            parsed_query
        };

        let top_docs = searcher
            .search(&*final_query, &TopDocs::with_limit(limit))
            .map_err(|e| HostError::storage(format!("Tantivy search failed: {e}")))?;

        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, doc_addr) in top_docs {
            let retrieved: TantivyDocument = searcher
                .doc(doc_addr)
                .map_err(|e| HostError::storage(format!("Tantivy doc fetch failed: {e}")))?;
            let message_id = retrieved
                .get_first(self.f_message_id)
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let cid = retrieved
                .get_first(self.f_chat_id)
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let sender_id = retrieved
                .get_first(self.f_sender_id)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let timestamp_ms = retrieved
                .get_first(self.f_timestamp_ms)
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let body = retrieved
                .get_first(self.f_body)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            hits.push(LocalSearchHit {
                message_id,
                chat_id: cid,
                sender_id,
                timestamp_ms,
                body,
                score,
                engine: "tantivy".to_string(),
            });
        }

        Ok(hits)
    }
}
