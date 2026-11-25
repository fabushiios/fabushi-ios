//! Tantivy 0.22 BM25 full-text & structured-operator community search indexer.

use gpui_telegram_core::{Result, TelegramError};
use gpui_telegram_proto::community_contracts::{
    ActorKind, CommunityPostDto, CommunitySearchHit, CommunitySearchRequestDto,
    CommunitySearchSort, CommunitySearchTab, PostStatus,
};
use parking_lot::Mutex;
use std::collections::HashSet;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, RangeQuery, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, Value, FAST, INDEXED, STORED, STRING, TEXT,
};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

const WRITER_HEAP_BYTES: usize = 15_000_000;

#[derive(Clone)]
struct CommunitySearchFields {
    schema: Schema,
    f_post_id: Field,
    f_community_slug: Field,
    f_author_id: Field,
    f_author_handle: Field,
    f_author_kind: Field,
    f_flair: Field,
    f_post_format: Field,
    f_has_media: Field,
    f_has_poll: Field,
    f_title: Field,
    f_body: Field,
    f_tags: Field,
    f_created_at_ms: Field,
    f_score: Field,
}

impl CommunitySearchFields {
    fn build() -> Self {
        let mut sb = Schema::builder();
        let f_post_id = sb.add_text_field("post_id", STRING | STORED);
        let f_community_slug = sb.add_text_field("community_slug", STRING | STORED);
        let f_author_id = sb.add_text_field("author_id", STRING | STORED);
        let f_author_handle = sb.add_text_field("author_handle", STRING | STORED);
        let f_author_kind = sb.add_text_field("author_kind", STRING | STORED);
        let f_flair = sb.add_text_field("flair", STRING | STORED);
        let f_post_format = sb.add_text_field("post_format", STRING | STORED);
        let f_has_media = sb.add_text_field("has_media", STRING | STORED);
        let f_has_poll = sb.add_text_field("has_poll", STRING | STORED);
        let f_title = sb.add_text_field("title", TEXT | STORED);
        let f_body = sb.add_text_field("body", TEXT | STORED);
        let f_tags = sb.add_text_field("tags", TEXT | STORED);
        let f_created_at_ms = sb.add_i64_field("created_at_ms", INDEXED | FAST | STORED);
        let f_score = sb.add_i64_field("score", INDEXED | FAST | STORED);
        Self {
            schema: sb.build(),
            f_post_id,
            f_community_slug,
            f_author_id,
            f_author_handle,
            f_author_kind,
            f_flair,
            f_post_format,
            f_has_media,
            f_has_poll,
            f_title,
            f_body,
            f_tags,
            f_created_at_ms,
            f_score,
        }
    }
}

/// Flattened document representation indexed into `CommunitySearchIndexer`.
#[derive(Debug, Clone)]
pub struct CommunityIndexDoc {
    pub post_id: String,
    pub community_slug: String,
    pub author_id: String,
    pub author_handle: String,
    pub author_kind: String,
    pub flair: String,
    pub post_format: String,
    pub has_media: bool,
    pub has_poll: bool,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub created_at_ms: i64,
    pub score: i64,
}

impl CommunityIndexDoc {
    pub fn from_post(post: &CommunityPostDto) -> Self {
        let mut tags = post.tags.clone();
        if let Some(f) = post.flair.as_deref().filter(|s| !s.trim().is_empty()) {
            tags.push(f.to_string());
        }
        Self {
            post_id: post.post_id.clone(),
            community_slug: normalize_slug_filter(
                post.community_slug.as_deref().unwrap_or(""),
            ),
            author_id: post.author_actor_id.clone(),
            author_handle: post
                .author_handle
                .trim()
                .trim_start_matches('@')
                .to_ascii_lowercase(),
            author_kind: post.author_kind.as_str().to_string(),
            flair: post
                .flair
                .as_deref()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase(),
            post_format: post.format.as_str().to_string(),
            has_media: !post.media.is_empty(),
            has_poll: post.poll.is_some(),
            title: post.title.clone().unwrap_or_default(),
            body: post.body.clone(),
            tags,
            created_at_ms: post.created_at_ms,
            score: post.score,
        }
    }
}

/// Parsed structured search query produced from `CommunitySearchRequestDto`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedCommunityQuery {
    pub free_terms: Vec<String>,
    pub community_slug: Option<String>,
    pub author_handle: Option<String>,
    pub flair: Option<String>,
    pub actor_kind: Option<String>,
    pub has_media: Option<bool>,
    pub has_poll: Option<bool>,
    pub post_format: Option<String>,
}

fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x3040..=0x309F
            | 0x30A0..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

fn normalize_slug_filter(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase();
    lower
        .strip_prefix("c/")
        .or_else(|| lower.strip_prefix("r/"))
        .unwrap_or(&lower)
        .to_string()
}

/// Inserts spaces at ASCII/CJK script boundaries and appends space-separated CJK unigrams & bigrams
/// so Tantivy's SimpleTokenizer matches both ASCII words and CJK substrings in mixed strings.
fn augment_cjk_bigrams(text: &str) -> String {
    let mut spaced = String::with_capacity(text.len() + 16);
    let mut cjk_tokens = Vec::new();
    let mut run = Vec::new();
    let mut prev_kind: u8 = 0; // 0 = separator/other, 1 = CJK, 2 = non-CJK alnum

    for ch in text.chars() {
        let cur_kind = if is_cjk(ch) {
            1
        } else if ch.is_alphanumeric() {
            2
        } else {
            0
        };
        if (prev_kind == 1 && cur_kind == 2) || (prev_kind == 2 && cur_kind == 1) {
            spaced.push(' ');
        }
        spaced.push(ch);
        prev_kind = cur_kind;

        if cur_kind == 1 {
            run.push(ch);
        } else if !run.is_empty() {
            emit_cjk_run(&run, &mut cjk_tokens);
            run.clear();
        }
    }
    if !run.is_empty() {
        emit_cjk_run(&run, &mut cjk_tokens);
    }
    if cjk_tokens.is_empty() {
        spaced
    } else {
        format!("{spaced} {}", cjk_tokens.join(" "))
    }
}

fn emit_cjk_run(run: &[char], out: &mut Vec<String>) {
    if run.len() == 1 {
        out.push(run[0].to_string());
        return;
    }
    for &c in run {
        out.push(c.to_string());
    }
    for pair in run.windows(2) {
        let mut s = String::with_capacity(8);
        s.push(pair[0]);
        s.push(pair[1]);
        out.push(s);
    }
}

fn tokenize_preserving_quotes(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in raw.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
            cur.push(ch);
        } else if ch.is_whitespace() && !in_quotes {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn sanitize_free_text_terms(raw: &str, out: &mut Vec<String>) {
    let mut cjk_run = Vec::new();
    let mut alnum_run = String::new();

    let flush_alnum = |buf: &mut String, dest: &mut Vec<String>| {
        if !buf.is_empty() {
            dest.push(std::mem::take(buf).to_lowercase());
        }
    };
    let flush_cjk = |run: &mut Vec<char>, dest: &mut Vec<String>| {
        if !run.is_empty() {
            if run.len() == 1 {
                dest.push(run[0].to_string());
            } else {
                for pair in run.windows(2) {
                    let mut bi = String::with_capacity(8);
                    bi.push(pair[0]);
                    bi.push(pair[1]);
                    dest.push(bi);
                }
            }
            run.clear();
        }
    };

    for ch in raw.chars() {
        if is_cjk(ch) {
            flush_alnum(&mut alnum_run, out);
            cjk_run.push(ch);
        } else if ch.is_alphanumeric() {
            flush_cjk(&mut cjk_run, out);
            for lc in ch.to_lowercase() {
                alnum_run.push(lc);
            }
        } else {
            flush_alnum(&mut alnum_run, out);
            flush_cjk(&mut cjk_run, out);
        }
    }
    flush_alnum(&mut alnum_run, out);
    flush_cjk(&mut cjk_run, out);
}

/// Parses structured operators and sanitizes free-text terms with zero parse errors on arbitrary input.
pub fn parse_community_search_query(req: &CommunitySearchRequestDto) -> ParsedCommunityQuery {
    let mut parsed = ParsedCommunityQuery::default();
    let mut raw_free = Vec::new();

    for tok in tokenize_preserving_quotes(&req.query) {
        if let Some((key_raw, val_raw)) = tok.split_once(':') {
            let key = key_raw.to_ascii_lowercase();
            let val = val_raw.trim_matches('"').trim();
            if !val.is_empty() {
                match key.as_str() {
                    "c" | "r" | "community" => {
                        let slug = normalize_slug_filter(val);
                        if !slug.is_empty() {
                            parsed.community_slug = Some(slug);
                            continue;
                        }
                    }
                    "author" | "from" => {
                        let handle = val.trim_start_matches('@').to_ascii_lowercase();
                        if !handle.is_empty() {
                            parsed.author_handle = Some(handle);
                            continue;
                        }
                    }
                    "flair" => {
                        parsed.flair = Some(val.to_ascii_lowercase());
                        continue;
                    }
                    "kind" | "actor" => {
                        if let Some(kind) = ActorKind::parse(val) {
                            parsed.actor_kind = Some(kind.as_str().to_string());
                            continue;
                        }
                    }
                    "has" => match val.to_ascii_lowercase().as_str() {
                        "media" | "image" | "video" => {
                            parsed.has_media = Some(true);
                            continue;
                        }
                        "poll" => {
                            parsed.has_poll = Some(true);
                            continue;
                        }
                        "link" | "url" => {
                            parsed.post_format = Some("link".to_string());
                            continue;
                        }
                        _ => {}
                    },
                    "format" => {
                        parsed.post_format = Some(val.to_ascii_lowercase());
                        continue;
                    }
                    _ => {}
                }
            }
        }
        raw_free.push(tok);
    }

    // Merge explicit DTO filters when not already specified inline.
    if parsed.community_slug.is_none() {
        parsed.community_slug = req
            .community_slug
            .as_deref()
            .map(normalize_slug_filter)
            .filter(|s| !s.is_empty());
    }
    if parsed.author_handle.is_none() {
        parsed.author_handle = req
            .author_handle
            .as_deref()
            .map(|s| s.trim().trim_start_matches('@').to_ascii_lowercase())
            .filter(|s| !s.is_empty());
    }
    if parsed.flair.is_none() {
        parsed.flair = req
            .flair
            .as_deref()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());
    }
    if parsed.actor_kind.is_none() {
        parsed.actor_kind = req.actor_kind.map(|k| k.as_str().to_string());
    }
    if parsed.has_media.is_none() {
        parsed.has_media = req.has_media;
    }
    if parsed.has_poll.is_none() {
        parsed.has_poll = req.has_poll;
    }
    match req.tab {
        CommunitySearchTab::Media => parsed.has_media = Some(true),
        CommunitySearchTab::Polls => parsed.has_poll = Some(true),
        CommunitySearchTab::Bots if parsed.actor_kind.is_none() => {
            parsed.actor_kind = Some("bot".to_string());
        }
        _ => {}
    }

    for token in raw_free {
        sanitize_free_text_terms(&token, &mut parsed.free_terms);
    }
    parsed
}

/// Thread-safe, zero-Tokio Tantivy 0.22 BM25 search engine for community posts.
#[derive(Clone)]
pub struct CommunitySearchIndexer {
    fields: CommunitySearchFields,
    index: Index,
    reader: IndexReader,
    writer: Arc<Mutex<IndexWriter>>,
    // Declared LAST so Tantivy files/locks close before any ephemeral TempDir drops.
    _ephemeral_dir: Option<Arc<tempfile::TempDir>>,
}

impl CommunitySearchIndexer {
    /// Creates an in-RAM Tantivy index with manual reload policy.
    pub fn in_ram() -> Result<Self> {
        let fields = CommunitySearchFields::build();
        let index = Index::create_in_ram(fields.schema.clone());
        Self::from_index(index, fields, None)
    }

    /// Opens an existing disk-backed index or creates a new one at `path`.
    /// Note: callers MUST NOT use path substrings `*tantivy*`, `*search_index*`, or `*fts_index*` (use e.g. `<db>.community.idx`).
    pub fn open_or_create(path: impl AsRef<Path>) -> Result<Self> {
        let dir_path = path.as_ref();
        std::fs::create_dir_all(dir_path)?;
        let fields = CommunitySearchFields::build();
        let dir = MmapDirectory::open(dir_path)
            .map_err(|e| TelegramError::Internal(format!("failed to open index dir: {e}")))?;
        let index = Index::open_or_create(dir, fields.schema.clone())
            .map_err(|e| TelegramError::Internal(format!("failed to open/create index: {e}")))?;
        Self::from_index(index, fields, None)
    }

    /// Opens an ephemeral RAII disk-backed index inside `std::env::temp_dir()`.
    pub fn open_ephemeral() -> Result<Self> {
        let tmp = tempfile::Builder::new()
            .prefix("gpui_tg_comm_idx_")
            .tempdir()?;
        let idx_path = tmp.path().join("community.idx");
        std::fs::create_dir_all(&idx_path)?;
        let fields = CommunitySearchFields::build();
        let dir = MmapDirectory::open(&idx_path)
            .map_err(|e| TelegramError::Internal(format!("failed to open ephemeral index dir: {e}")))?;
        let index = Index::open_or_create(dir, fields.schema.clone())
            .map_err(|e| TelegramError::Internal(format!("failed to create ephemeral index: {e}")))?;
        Self::from_index(index, fields, Some(Arc::new(tmp)))
    }

    fn from_index(
        index: Index,
        fields: CommunitySearchFields,
        ephemeral_dir: Option<Arc<tempfile::TempDir>>,
    ) -> Result<Self> {
        let mut last_err = None;
        let mut writer_opt = None;
        for attempt in 0..10 {
            match index.writer::<TantivyDocument>(WRITER_HEAP_BYTES) {
                Ok(w) => {
                    writer_opt = Some(w);
                    break;
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 9 {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
            }
        }
        let writer = writer_opt.ok_or_else(|| {
            TelegramError::Internal(format!(
                "failed to acquire community search writer lock: {:?}",
                last_err
            ))
        })?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|e| TelegramError::Internal(format!("failed to create index reader: {e}")))?;

        Ok(Self {
            fields,
            index,
            reader,
            writer: Arc::new(Mutex::new(writer)),
            _ephemeral_dir: ephemeral_dir,
        })
    }

    /// Upserts a `CommunityPostDto` into the index (or deletes it if unpublished/removed).
    pub fn index_post(&self, post: &CommunityPostDto) -> Result<()> {
        if post.status != PostStatus::Published || post.is_removed {
            return self.delete_post(&post.post_id);
        }
        let doc = CommunityIndexDoc::from_post(post);
        self.index_doc(&doc)
    }

    /// Upserts a flattened `CommunityIndexDoc` by `post_id`.
    pub fn index_doc(&self, doc: &CommunityIndexDoc) -> Result<()> {
        let writer = self.writer.lock();
        writer.delete_term(Term::from_field_text(self.fields.f_post_id, &doc.post_id));

        let mut tdoc = TantivyDocument::default();
        tdoc.add_text(self.fields.f_post_id, &doc.post_id);
        tdoc.add_text(self.fields.f_community_slug, &doc.community_slug.to_ascii_lowercase());
        tdoc.add_text(self.fields.f_author_id, &doc.author_id);
        tdoc.add_text(self.fields.f_author_handle, &doc.author_handle.to_ascii_lowercase());
        tdoc.add_text(self.fields.f_author_kind, &doc.author_kind.to_ascii_lowercase());
        tdoc.add_text(self.fields.f_flair, &doc.flair.to_ascii_lowercase());
        tdoc.add_text(self.fields.f_post_format, &doc.post_format.to_ascii_lowercase());
        tdoc.add_text(self.fields.f_has_media, if doc.has_media { "1" } else { "0" });
        tdoc.add_text(self.fields.f_has_poll, if doc.has_poll { "1" } else { "0" });
        tdoc.add_text(self.fields.f_title, augment_cjk_bigrams(&doc.title));
        tdoc.add_text(self.fields.f_body, augment_cjk_bigrams(&doc.body));
        let tags_joined = augment_cjk_bigrams(&doc.tags.join(" "));
        tdoc.add_text(self.fields.f_tags, tags_joined);
        tdoc.add_i64(self.fields.f_created_at_ms, doc.created_at_ms);
        tdoc.add_i64(self.fields.f_score, doc.score);

        writer
            .add_document(tdoc)
            .map_err(|e| TelegramError::Internal(format!("failed to add search doc: {e}")))?;
        Ok(())
    }

    /// Deletes a post from the index by `post_id`.
    pub fn delete_post(&self, post_id: &str) -> Result<()> {
        let writer = self.writer.lock();
        writer.delete_term(Term::from_field_text(self.fields.f_post_id, post_id));
        Ok(())
    }

    /// Commits pending mutations and reloads the reader for immediate search visibility.
    pub fn commit(&self) -> Result<()> {
        let mut writer = self.writer.lock();
        writer
            .commit()
            .map_err(|e| TelegramError::Internal(format!("failed to commit search index: {e}")))?;
        drop(writer);
        self.reader
            .reload()
            .map_err(|e| TelegramError::Internal(format!("failed to reload search reader: {e}")))?;
        Ok(())
    }

    /// Returns the number of live committed documents in the index.
    pub fn doc_count(&self) -> u64 {
        self.reader.searcher().num_docs()
    }
}

impl CommunitySearchIndexer {
    /// Executes a search returning ordered matching `post_id` strings.
    pub fn search(&self, req: &CommunitySearchRequestDto) -> Result<Vec<String>> {
        Ok(self
            .search_hits(req)?
            .into_iter()
            .map(|h| h.post_id)
            .collect())
    }

    /// Executes a BM25 + structured-operator search returning scored `CommunitySearchHit` entries.
    pub fn search_hits(&self, req: &CommunitySearchRequestDto) -> Result<Vec<CommunitySearchHit>> {
        let parsed = parse_community_search_query(req);
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        if let Some(slug) = parsed.community_slug.as_deref() {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.f_community_slug, slug),
                    IndexRecordOption::Basic,
                )),
            ));
        }
        if let Some(author) = parsed.author_handle.as_deref() {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.f_author_handle, author),
                    IndexRecordOption::Basic,
                )),
            ));
        }
        if let Some(flair) = parsed.flair.as_deref() {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.f_flair, flair),
                    IndexRecordOption::Basic,
                )),
            ));
        }
        if let Some(kind) = parsed.actor_kind.as_deref() {
            if kind == "bot" {
                let bot_q: Box<dyn Query> = Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.f_author_kind, "bot"),
                    IndexRecordOption::Basic,
                ));
                let hybrid_q: Box<dyn Query> = Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.f_author_kind, "hybrid_agent"),
                    IndexRecordOption::Basic,
                ));
                clauses.push((
                    Occur::Must,
                    Box::new(BooleanQuery::new(vec![
                        (Occur::Should, bot_q),
                        (Occur::Should, hybrid_q),
                    ])),
                ));
            } else {
                clauses.push((
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(self.fields.f_author_kind, kind),
                        IndexRecordOption::Basic,
                    )),
                ));
            }
        }
        if let Some(has_media) = parsed.has_media {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.f_has_media, if has_media { "1" } else { "0" }),
                    IndexRecordOption::Basic,
                )),
            ));
        }
        if let Some(has_poll) = parsed.has_poll {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.f_has_poll, if has_poll { "1" } else { "0" }),
                    IndexRecordOption::Basic,
                )),
            ));
        }
        if let Some(fmt) = parsed.post_format.as_deref() {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.f_post_format, fmt),
                    IndexRecordOption::Basic,
                )),
            ));
        }

        if !parsed.free_terms.is_empty() {
            let mut qp = QueryParser::for_index(
                &self.index,
                vec![
                    self.fields.f_title,
                    self.fields.f_tags,
                    self.fields.f_body,
                    self.fields.f_author_handle,
                ],
            );
            qp.set_field_boost(self.fields.f_title, 2.5);
            qp.set_field_boost(self.fields.f_tags, 2.0);
            qp.set_field_boost(self.fields.f_author_handle, 1.5);
            let sanitized_str = parsed.free_terms.join(" ");
            let text_query: Box<dyn Query> = match qp.parse_query(&sanitized_str) {
                Ok(q) => q,
                Err(_) => {
                    let mut fallback = Vec::new();
                    for t in &parsed.free_terms {
                        fallback.push((
                            Occur::Should,
                            Box::new(TermQuery::new(
                                Term::from_field_text(self.fields.f_body, t),
                                IndexRecordOption::Basic,
                            )) as Box<dyn Query>,
                        ));
                    }
                    Box::new(BooleanQuery::new(fallback))
                }
            };
            clauses.push((Occur::Must, text_query));
        }

        // Empty free-text AND no structured filters -> empty result (never triggers EmptyQuery panic)
        if clauses.is_empty() {
            return Ok(Vec::new());
        }

        let effective_sort = if req.tab == CommunitySearchTab::Latest {
            CommunitySearchSort::Latest
        } else {
            req.sort
        };
        let final_query = BooleanQuery::new(clauses);
        let searcher = self.reader.searcher();
        let total_index_docs = searcher.num_docs() as usize;
        if total_index_docs == 0 || req.offset >= total_index_docs {
            return Ok(Vec::new());
        }
        let limit = req.limit.clamp(1, 100);
        let max_scan = total_index_docs.min(50_000).max(1);
        let fetch_limit = req.offset.saturating_add(limit).min(max_scan).max(1);
        let probe_limit = fetch_limit.saturating_add(1).min(max_scan);

        let mut hits = Vec::new();
        match effective_sort {
            CommunitySearchSort::Relevance => {
                let mut top_docs = searcher
                    .search(&final_query, &TopDocs::with_limit(probe_limit))
                    .map_err(|e| TelegramError::Internal(format!("search execution failed: {e}")))?;
                let tie_spans_boundary = top_docs.len() > fetch_limit
                    && (top_docs[fetch_limit].0 - top_docs[fetch_limit - 1].0).abs() <= 1e-6;
                if tie_spans_boundary {
                    top_docs = searcher
                        .search(&final_query, &TopDocs::with_limit(max_scan))
                        .map_err(|e| {
                            TelegramError::Internal(format!("search tie-expansion failed: {e}"))
                        })?;
                } else {
                    top_docs.truncate(fetch_limit);
                }
                hits.reserve(top_docs.len());
                for (bm25_score, doc_addr) in top_docs {
                    if let Some(hit) = self.doc_to_hit(&searcher, doc_addr, bm25_score)? {
                        hits.push(hit);
                    }
                }
                hits.sort_by(|a, b| {
                    let sa = if a.bm25_score.is_finite() {
                        a.bm25_score
                    } else {
                        f32::NEG_INFINITY
                    };
                    let sb = if b.bm25_score.is_finite() {
                        b.bm25_score
                    } else {
                        f32::NEG_INFINITY
                    };
                    sb.total_cmp(&sa)
                        .then_with(|| b.post_score.cmp(&a.post_score))
                        .then_with(|| b.created_at_ms.cmp(&a.created_at_ms))
                        .then_with(|| a.post_id.cmp(&b.post_id))
                });
            }
            CommunitySearchSort::Latest | CommunitySearchSort::Top => {
                let field_name = match effective_sort {
                    CommunitySearchSort::Latest => "created_at_ms",
                    CommunitySearchSort::Top => "score",
                    CommunitySearchSort::Relevance => unreachable!(),
                };
                let collector = TopDocs::with_limit(probe_limit)
                    .order_by_fast_field::<i64>(field_name, tantivy::Order::Desc);
                let mut top_docs = searcher
                    .search(&final_query, &collector)
                    .map_err(|e| TelegramError::Internal(format!("search {field_name} failed: {e}")))?;
                let tie_spans_boundary = top_docs.len() > fetch_limit
                    && top_docs[fetch_limit].0 == top_docs[fetch_limit - 1].0;

                let doc_addrs: Vec<tantivy::DocAddress> = if tie_spans_boundary {
                    let boundary = top_docs[fetch_limit - 1].0;
                    let tie_range = RangeQuery::new_i64_bounds(
                        field_name.to_string(),
                        Bound::Included(boundary),
                        Bound::Included(boundary),
                    );
                    let tie_query = BooleanQuery::new(vec![
                        (Occur::Must, Box::new(final_query.clone())),
                        (Occur::Must, Box::new(tie_range)),
                    ]);
                    let tie_docs = searcher
                        .search(&tie_query, &TopDocs::with_limit(max_scan))
                        .map_err(|e| {
                            TelegramError::Internal(format!("search tie bucket failed: {e}"))
                        })?;
                    let mut seen: HashSet<tantivy::DocAddress> =
                        HashSet::with_capacity(fetch_limit + tie_docs.len());
                    let mut merged = Vec::with_capacity(fetch_limit + tie_docs.len());
                    for (val, addr) in top_docs {
                        if val >= boundary && seen.insert(addr) {
                            merged.push(addr);
                        }
                    }
                    for (_score, addr) in tie_docs {
                        if seen.insert(addr) {
                            merged.push(addr);
                        }
                    }
                    merged
                } else {
                    top_docs.truncate(fetch_limit);
                    top_docs.into_iter().map(|(_val, addr)| addr).collect()
                };

                hits.reserve(doc_addrs.len());
                for doc_addr in doc_addrs {
                    if let Some(hit) = self.doc_to_hit(&searcher, doc_addr, 1.0)? {
                        hits.push(hit);
                    }
                }
                match effective_sort {
                    CommunitySearchSort::Latest => {
                        hits.sort_by(|a, b| {
                            b.created_at_ms
                                .cmp(&a.created_at_ms)
                                .then_with(|| b.post_score.cmp(&a.post_score))
                                .then_with(|| a.post_id.cmp(&b.post_id))
                        });
                    }
                    CommunitySearchSort::Top => {
                        hits.sort_by(|a, b| {
                            b.post_score
                                .cmp(&a.post_score)
                                .then_with(|| b.created_at_ms.cmp(&a.created_at_ms))
                                .then_with(|| a.post_id.cmp(&b.post_id))
                        });
                    }
                    CommunitySearchSort::Relevance => unreachable!(),
                }
            }
        }

        Ok(hits.into_iter().skip(req.offset).take(limit).collect())
    }

    fn doc_to_hit(
        &self,
        searcher: &tantivy::Searcher,
        doc_addr: tantivy::DocAddress,
        bm25_score: f32,
    ) -> Result<Option<CommunitySearchHit>> {
        let doc: TantivyDocument = searcher
            .doc(doc_addr)
            .map_err(|e| TelegramError::Internal(format!("failed to load doc: {e}")))?;
        let Some(post_id) = doc
            .get_first(self.fields.f_post_id)
            .and_then(|v| v.as_str())
            .map(str::to_string)
        else {
            return Ok(None);
        };
        let created_at_ms = doc
            .get_first(self.fields.f_created_at_ms)
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let post_score = doc
            .get_first(self.fields.f_score)
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        Ok(Some(CommunitySearchHit {
            post_id,
            bm25_score,
            created_at_ms,
            post_score,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seed::build_starter_dataset;

    #[test]
    fn in_ram_search_operators_cjk_and_adversarial_punctuation() {
        let now = 1_760_100_000_000_i64;
        let ds = build_starter_dataset(now);
        let indexer = CommunitySearchIndexer::in_ram().unwrap();
        for p in &ds.posts {
            indexer.index_post(p).unwrap();
        }
        indexer.commit().unwrap();
        assert!(indexer.doc_count() >= 24);

        // 1. Operator-only query (must avoid Tantivy EmptyQuery trap!)
        let bot_rust = indexer
            .search(&CommunitySearchRequestDto {
                query: "c:rust kind:bot".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(bot_rust.contains(&"seed_post_10".to_string()));
        assert!(!bot_rust.contains(&"seed_post_02".to_string()));

        // 2. Quoted flair + author operator
        let deep_dive = indexer
            .search(&CommunitySearchRequestDto {
                query: r#"author:alex_chen flair:"Deep Dive""#.into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(deep_dive, vec!["seed_post_02".to_string()]);

        // 3. has:media and has:poll
        let media_hits = indexer
            .search(&CommunitySearchRequestDto {
                query: "has:media".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(media_hits.contains(&"seed_post_11".to_string()));

        let poll_hits = indexer
            .search(&CommunitySearchRequestDto {
                query: "has:poll".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(poll_hits.len() >= 3);

        // 4. CJK bigram query
        let cjk_hits = indexer
            .search(&CommunitySearchRequestDto {
                query: "推荐算法".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(cjk_hits.contains(&"seed_post_18".to_string()));

        // 5. Adversarial punctuation must NEVER error
        let adv = indexer.search(&CommunitySearchRequestDto {
            query: r#"std::sync::Arc c++ [unclosed (foo:bar "baz !@#$%^&*()"#.into(),
            ..Default::default()
        });
        assert!(adv.is_ok());

        let punct_only = indexer
            .search(&CommunitySearchRequestDto {
                query: "!!! ::: +++ ---".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(punct_only.is_empty());

        // 6. Delete removes post after commit
        indexer.delete_post("seed_post_02").unwrap();
        indexer.commit().unwrap();
        let after_del = indexer
            .search(&CommunitySearchRequestDto {
                query: r#"author:alex_chen flair:"Deep Dive""#.into(),
                ..Default::default()
            })
            .unwrap();
        assert!(after_del.is_empty());
    }

    #[test]
    fn disk_index_reopen_persistence_and_tempdir_cleanup() {
        let tmp = tempfile::Builder::new()
            .prefix("gpui_tg_comm_idx_test_")
            .tempdir()
            .unwrap();
        let idx_dir = tmp.path().join("test.community.idx");
        let ds = build_starter_dataset(1_760_100_000_000);
        {
            let indexer = CommunitySearchIndexer::open_or_create(&idx_dir).unwrap();
            indexer.index_post(&ds.posts[0]).unwrap();
            indexer.index_post(&ds.posts[1]).unwrap();
            indexer.commit().unwrap();
            assert_eq!(indexer.doc_count(), 2);
        }
        {
            let reopened = CommunitySearchIndexer::open_or_create(&idx_dir).unwrap();
            assert_eq!(reopened.doc_count(), 2);
            let hits = reopened
                .search(&CommunitySearchRequestDto {
                    query: "GPUI 120fps".into(),
                    ..Default::default()
                })
                .unwrap();
            assert!(hits.contains(&"seed_post_02".to_string()));
        }
        let dir_copy = tmp.path().to_path_buf();
        drop(tmp);
        assert!(!dir_copy.exists(), "TempDir should be cleaned up on drop");
    }

    #[test]
    fn fast_field_top_latest_sort_mixed_ascii_cjk_and_uppercase_prefix() {
        let indexer = CommunitySearchIndexer::in_ram().unwrap();
        let base_ts = 1_760_000_000_000_i64;
        for i in 0..15i64 {
            let doc = CommunityIndexDoc {
                post_id: format!("sort_post_{i:02}"),
                community_slug: "rust".into(),
                author_id: "u:1".into(),
                author_handle: "alex".into(),
                author_kind: "human".into(),
                flair: "discussion".into(),
                post_format: "micro".into(),
                has_media: false,
                has_poll: false,
                title: if i == 14 {
                    "Tokio调度器优化指南".into()
                } else {
                    format!("Post number {i}")
                },
                body: format!("General Rust content #{i}"),
                tags: vec!["rust".into()],
                created_at_ms: base_ts + i * 1_000,
                score: if i == 14 { 9999 } else { i * 10 },
            };
            indexer.index_doc(&doc).unwrap();
        }
        indexer.commit().unwrap();

        // 1. Operator-only query with Top sort & limit=2 MUST return sort_post_14 first!
        let top_hits = indexer
            .search(&CommunitySearchRequestDto {
                query: "c:rust".into(),
                sort: CommunitySearchSort::Top,
                limit: 2,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(top_hits, vec!["sort_post_14", "sort_post_13"]);

        // 2. Operator-only query with Latest sort & limit=2 MUST return newest docs!
        let latest_hits = indexer
            .search(&CommunitySearchRequestDto {
                query: "c:C/Rust".into(),
                sort: CommunitySearchSort::Latest,
                limit: 2,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(latest_hits, vec!["sort_post_14", "sort_post_13"]);

        // 3. Explicit community_slug: Some("C/Rust") works identically
        let dto_comm_hits = indexer
            .search(&CommunitySearchRequestDto {
                query: "Tokio".into(),
                community_slug: Some("C/Rust".into()),
                limit: 5,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(dto_comm_hits, vec!["sort_post_14"]);

        // 4. Mixed ASCII+CJK string matches ASCII-only, CJK-only, and mixed queries
        let cjk_hits = indexer
            .search(&CommunitySearchRequestDto {
                query: "调度器".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(cjk_hits, vec!["sort_post_14"]);

        let mixed_hits = indexer
            .search(&CommunitySearchRequestDto {
                query: "Tokio调度器".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(mixed_hits, vec!["sort_post_14"]);
    }

    #[test]
    fn tie_pagination_is_duplicate_free_and_complete_across_top_latest_and_relevance() {
        let indexer = CommunitySearchIndexer::in_ram().unwrap();
        let base_ts = 1_760_000_000_000_i64;

        // 1. 8 docs in c/rust with IDENTICAL score = 42, strictly increasing created_at_ms
        for i in 0..8i64 {
            indexer
                .index_doc(&CommunityIndexDoc {
                    post_id: format!("tie_score_{i:02}"),
                    community_slug: "rust".into(),
                    author_id: "u:1".into(),
                    author_handle: "alex".into(),
                    author_kind: "human".into(),
                    flair: "discussion".into(),
                    post_format: "micro".into(),
                    has_media: false,
                    has_poll: false,
                    title: format!("Tied score {i}"),
                    body: "Common tie body".into(),
                    tags: vec!["rust".into()],
                    created_at_ms: base_ts + i * 1_000,
                    score: 42,
                })
                .unwrap();
        }

        // 2. 8 docs in c/systems with IDENTICAL created_at_ms, identical score, reverse-alphabetical post_ids
        for (idx, suffix) in ['h', 'g', 'f', 'e', 'd', 'c', 'b', 'a'].iter().enumerate() {
            indexer
                .index_doc(&CommunityIndexDoc {
                    post_id: format!("tie_ts_{suffix}"),
                    community_slug: "systems".into(),
                    author_id: "u:2".into(),
                    author_handle: "maya".into(),
                    author_kind: "human".into(),
                    flair: "systems".into(),
                    post_format: "micro".into(),
                    has_media: false,
                    has_poll: false,
                    title: format!("Tied ts {idx}"),
                    body: "Systems tie body".into(),
                    tags: vec!["systems".into()],
                    created_at_ms: base_ts + 500_000,
                    score: 10,
                })
                .unwrap();
        }

        // 3. 6 docs in c/neg_ties with IDENTICAL negative score = -50
        for i in 0..6i64 {
            indexer
                .index_doc(&CommunityIndexDoc {
                    post_id: format!("neg_tie_{i:02}"),
                    community_slug: "neg_ties".into(),
                    author_id: "u:3".into(),
                    author_handle: "bot".into(),
                    author_kind: "bot".into(),
                    flair: "".into(),
                    post_format: "micro".into(),
                    has_media: false,
                    has_poll: false,
                    title: format!("Neg tie {i}"),
                    body: "Negative tied score".into(),
                    tags: vec![],
                    created_at_ms: base_ts + i * 2_000,
                    score: -50,
                })
                .unwrap();
        }
        indexer.commit().unwrap();

        // Top pagination: limit=2 across offset=0,2,4,6 -> exact descending created_at_ms order, all 8 unique
        let mut top_all = Vec::new();
        for offset in [0, 2, 4, 6] {
            let page = indexer
                .search(&CommunitySearchRequestDto {
                    query: "c:rust".into(),
                    sort: CommunitySearchSort::Top,
                    limit: 2,
                    offset,
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(page.len(), 2);
            top_all.extend(page);
        }
        assert_eq!(
            top_all,
            vec![
                "tie_score_07",
                "tie_score_06",
                "tie_score_05",
                "tie_score_04",
                "tie_score_03",
                "tie_score_02",
                "tie_score_01",
                "tie_score_00",
            ]
        );

        // Latest pagination on identical created_at_ms: limit=2 across offset=0,2,4,6 -> all 8 unique in post_id ASC order
        let mut latest_all = Vec::new();
        for offset in [0, 2, 4, 6] {
            let page = indexer
                .search(&CommunitySearchRequestDto {
                    query: "c:systems".into(),
                    sort: CommunitySearchSort::Latest,
                    limit: 2,
                    offset,
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(page.len(), 2);
            latest_all.extend(page);
        }
        assert_eq!(
            latest_all,
            vec![
                "tie_ts_a", "tie_ts_b", "tie_ts_c", "tie_ts_d", "tie_ts_e", "tie_ts_f",
                "tie_ts_g", "tie_ts_h",
            ]
        );

        // Relevance pagination on operator-only query (identical BM25 == 1.0): all 8 unique
        let mut rel_all = Vec::new();
        for offset in [0, 2, 4, 6] {
            let page = indexer
                .search(&CommunitySearchRequestDto {
                    query: "c:rust".into(),
                    sort: CommunitySearchSort::Relevance,
                    limit: 2,
                    offset,
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(page.len(), 2);
            rel_all.extend(page);
        }
        assert_eq!(rel_all, top_all);

        // Negative tied scores (-50) paginate cleanly across all 6 unique docs
        let mut neg_all = Vec::new();
        for offset in [0, 2, 4] {
            let page = indexer
                .search(&CommunitySearchRequestDto {
                    query: "c:neg_ties".into(),
                    sort: CommunitySearchSort::Top,
                    limit: 2,
                    offset,
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(page.len(), 2);
            neg_all.extend(page);
        }
        assert_eq!(
            neg_all,
            vec![
                "neg_tie_05",
                "neg_tie_04",
                "neg_tie_03",
                "neg_tie_02",
                "neg_tie_01",
                "neg_tie_00",
            ]
        );

        // Huge offset returns empty cleanly without panic
        let empty = indexer
            .search(&CommunitySearchRequestDto {
                query: "c:rust".into(),
                sort: CommunitySearchSort::Top,
                limit: 10,
                offset: usize::MAX,
                ..Default::default()
            })
            .unwrap();
        assert!(empty.is_empty());
    }
}
