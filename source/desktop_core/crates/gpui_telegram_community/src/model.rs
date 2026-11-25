//! Communities, Posts, Comments validation, entity extraction, search operators, and poll tallies.

use gpui_telegram_core::{Result, TelegramError};
use gpui_telegram_proto::community_contracts::{
    ActorKind, CreateCommentRequest, CreatePostRequest, PollDto, PostFormat, PostStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MIN_SLUG_LEN: usize = 2;
pub const MAX_SLUG_LEN: usize = 32;
pub const MAX_MICRO_BODY_CHARS: usize = 2_000;
pub const MAX_ARTICLE_BODY_CHARS: usize = 40_000;
pub const MAX_COMMENT_BODY_CHARS: usize = 10_000;
pub const MIN_POLL_DURATION_SECS: i64 = 300;
pub const MAX_POLL_DURATION_SECS: i64 = 2_592_000;

/// Normalizes and validates a community slug (`c/<slug>` -> `<slug>`, `2..=32` `[a-z0-9_]`).
pub fn normalize_community_slug(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix("c/")
        .or_else(|| trimmed.strip_prefix("C/"))
        .or_else(|| trimmed.strip_prefix("r/"))
        .or_else(|| trimmed.strip_prefix("R/"))
        .unwrap_or(trimmed)
        .to_ascii_lowercase();
    if stripped.len() < MIN_SLUG_LEN || stripped.len() > MAX_SLUG_LEN {
        return Err(TelegramError::Validation(format!(
            "community slug must be {MIN_SLUG_LEN}..={MAX_SLUG_LEN} characters"
        )));
    }
    if !stripped
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(TelegramError::Validation(
            "community slug may only contain ASCII letters, digits, and underscores".to_string(),
        ));
    }
    Ok(stripped)
}

/// Validates a `CreatePostRequest` across all 8 unified Reddit + X formats.
pub fn validate_create_post_request(req: &CreatePostRequest, _now_ms: i64) -> Result<()> {
    if let Some(slug) = req.community_slug.as_deref() {
        if !slug.trim().is_empty() {
            normalize_community_slug(slug)?;
        }
    }
    if req.status == PostStatus::Deleted {
        return Err(TelegramError::Validation(
            "cannot create a post with status 'deleted'".to_string(),
        ));
    }
    if req.status == PostStatus::Scheduled && req.scheduled_at_ms.is_none() {
        return Err(TelegramError::Validation(
            "scheduled posts must specify scheduled_at_ms".to_string(),
        ));
    }

    let body_trimmed = req.body.trim();
    let body_chars = body_trimmed.chars().count();
    if body_chars > MAX_ARTICLE_BODY_CHARS {
        return Err(TelegramError::Validation(format!(
            "post body exceeds {MAX_ARTICLE_BODY_CHARS} characters"
        )));
    }
    let title_trimmed = req.title.as_deref().map(str::trim).unwrap_or("");
    if title_trimmed.chars().count() > 300 {
        return Err(TelegramError::Validation(
            "post title must be <= 300 characters".to_string(),
        ));
    }

    match req.format {
        PostFormat::Article => {
            if title_trimmed.is_empty() {
                return Err(TelegramError::Validation(
                    "article posts require a non-empty title".to_string(),
                ));
            }
            if body_trimmed.is_empty() || body_chars > MAX_ARTICLE_BODY_CHARS {
                return Err(TelegramError::Validation(format!(
                    "article body must be 1..={MAX_ARTICLE_BODY_CHARS} characters"
                )));
            }
        }
        PostFormat::Micro => {
            if body_chars > MAX_MICRO_BODY_CHARS {
                return Err(TelegramError::Validation(format!(
                    "micro post body must be <= {MAX_MICRO_BODY_CHARS} characters"
                )));
            }
            if body_trimmed.is_empty()
                && req.media.is_empty()
                && req.poll.is_none()
                && req.quoted_post_id.is_none()
            {
                return Err(TelegramError::Validation(
                    "micro post must contain text, media, a poll, or a quoted post".to_string(),
                ));
            }
        }
        PostFormat::Media => {
            if req.media.is_empty() || req.media.len() > 4 {
                return Err(TelegramError::Validation(
                    "media posts must attach 1..=4 items".to_string(),
                ));
            }
            if body_chars > MAX_ARTICLE_BODY_CHARS {
                return Err(TelegramError::Validation(format!(
                    "post body exceeds {MAX_ARTICLE_BODY_CHARS} characters"
                )));
            }
        }
        PostFormat::Link => {
            let has_url_in_body = !extract_entities(body_trimmed).urls.is_empty();
            if req.link_preview.is_none() && !has_url_in_body {
                return Err(TelegramError::Validation(
                    "link posts require a link_preview or URL in body".to_string(),
                ));
            }
        }
        PostFormat::Poll => {
            let poll = req.poll.as_ref().ok_or_else(|| {
                TelegramError::Validation("poll posts require a poll payload".to_string())
            })?;
            if poll.question.trim().is_empty() {
                return Err(TelegramError::Validation(
                    "poll question cannot be empty".to_string(),
                ));
            }
            if !(2..=6).contains(&poll.options.len()) {
                return Err(TelegramError::Validation(
                    "polls must have between 2 and 6 options".to_string(),
                ));
            }
            let mut seen = HashSet::new();
            for opt in &poll.options {
                let t = opt.trim();
                if t.is_empty() || t.chars().count() > 140 {
                    return Err(TelegramError::Validation(
                        "each poll option must be 1..=140 characters".to_string(),
                    ));
                }
                if !seen.insert(t.to_ascii_lowercase()) {
                    return Err(TelegramError::Validation(
                        "poll options must be distinct".to_string(),
                    ));
                }
            }
            let dur = if poll.duration_secs == 0 {
                86_400
            } else {
                poll.duration_secs
            };
            if !(MIN_POLL_DURATION_SECS..=MAX_POLL_DURATION_SECS).contains(&dur) {
                return Err(TelegramError::Validation(format!(
                    "poll duration must be {MIN_POLL_DURATION_SECS}..={MAX_POLL_DURATION_SECS} seconds"
                )));
            }
        }
        PostFormat::Quote => {
            if req
                .quoted_post_id
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                return Err(TelegramError::Validation(
                    "quote posts require quoted_post_id".to_string(),
                ));
            }
        }
        PostFormat::Repost => {
            if req
                .reposted_post_id
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                return Err(TelegramError::Validation(
                    "repost requires reposted_post_id".to_string(),
                ));
            }
        }
        PostFormat::Crosspost => {
            if req
                .crosspost_of_id
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                return Err(TelegramError::Validation(
                    "crosspost requires crosspost_of_id".to_string(),
                ));
            }
        }
    }

    if req.media.len() > 4 {
        return Err(TelegramError::Validation(
            "posts may attach at most 4 media items".to_string(),
        ));
    }
    for part in &req.thread_parts {
        if part.body.trim().is_empty() && part.media.is_empty() {
            return Err(TelegramError::Validation(
                "each thread part must contain body text or media".to_string(),
            ));
        }
        if part.body.chars().count() > MAX_MICRO_BODY_CHARS {
            return Err(TelegramError::Validation(format!(
                "thread part body must be <= {MAX_MICRO_BODY_CHARS} characters"
            )));
        }
    }
    Ok(())
}

/// Validates a `CreateCommentRequest`.
pub fn validate_create_comment_request(req: &CreateCommentRequest) -> Result<()> {
    let trimmed = req.body.trim();
    if trimmed.is_empty() && req.media.is_empty() {
        return Err(TelegramError::Validation(
            "comment must contain text or media".to_string(),
        ));
    }
    if trimmed.chars().count() > MAX_COMMENT_BODY_CHARS {
        return Err(TelegramError::Validation(format!(
            "comment body exceeds {MAX_COMMENT_BODY_CHARS} characters"
        )));
    }
    if req.media.len() > 4 {
        return Err(TelegramError::Validation(
            "comments may attach at most 4 media items".to_string(),
        ));
    }
    Ok(())
}

/// Extracted structured entities from post or comment Markdown/text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedEntities {
    pub mentions: Vec<String>,
    pub hashtags: Vec<String>,
    pub communities: Vec<String>,
    pub urls: Vec<String>,
}

fn push_unique(target: &mut Vec<String>, seen: &mut HashSet<String>, value: String) {
    if !value.is_empty() && seen.insert(value.clone()) {
        target.push(value);
    }
}

fn is_mention_boundary_prev(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(c) => !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-')),
    }
}

fn is_hashtag_boundary_prev(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(c) => !(c.is_alphanumeric() || c == '_'),
    }
}

fn is_community_boundary_prev(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(c) => !(c.is_alphanumeric() || c == '_' || c == '/' || c == ':'),
    }
}

/// Extracts `@mentions` (excluding emails), `#hashtags` (including Unicode/CJK), `c/<slug>`, and URLs.
pub fn extract_entities(text: &str) -> ExtractedEntities {
    let mut out = ExtractedEntities::default();
    let mut seen_mentions = HashSet::new();
    let mut seen_hashtags = HashSet::new();
    let mut seen_communities = HashSet::new();
    let mut seen_urls = HashSet::new();

    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let (byte_idx, ch) = chars[i];
        let prev = if i > 0 { Some(chars[i - 1].1) } else { None };

        // 1. URLs: http:// or https://
        if (text[byte_idx..].starts_with("https://") || text[byte_idx..].starts_with("http://"))
            && prev.is_none_or(|c| !c.is_ascii_alphanumeric())
        {
            let mut j = i;
            while j < len {
                let c = chars[j].1;
                if !c.is_ascii()
                    || c.is_ascii_whitespace()
                    || c.is_ascii_control()
                    || matches!(c, '<' | '>' | '"')
                {
                    break;
                }
                j += 1;
            }
            let end_byte = if j < len { chars[j].0 } else { text.len() };
            let raw_url = &text[byte_idx..end_byte];
            let trimmed = raw_url.trim_end_matches(|c: char| {
                matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '>' | '"' | '\'')
            });
            if trimmed.len() > "https://".len() || (trimmed.starts_with("http://") && trimmed.len() > 7) {
                push_unique(&mut out.urls, &mut seen_urls, trimmed.to_string());
            }
            i = j;
            continue;
        }

        // 2. @mentions (reject email addresses via boundary check)
        if ch == '@' && is_mention_boundary_prev(prev) {
            let mut j = i + 1;
            let mut handle = String::new();
            while j < len {
                let c = chars[j].1;
                if c.is_ascii_alphanumeric() || c == '_' {
                    handle.push(c.to_ascii_lowercase());
                    j += 1;
                } else {
                    break;
                }
            }
            if (2..=32).contains(&handle.len()) && !handle.chars().all(|c| c == '_') {
                push_unique(&mut out.mentions, &mut seen_mentions, handle);
                i = j;
                continue;
            }
        }

        // 3. #hashtags (ASCII + Unicode/CJK letters & digits)
        if ch == '#' && is_hashtag_boundary_prev(prev) {
            let mut j = i + 1;
            let mut tag = String::new();
            while j < len {
                let c = chars[j].1;
                if c.is_alphanumeric() || c == '_' {
                    for lc in c.to_lowercase() {
                        tag.push(lc);
                    }
                    j += 1;
                } else {
                    break;
                }
            }
            let tag_chars = tag.chars().count();
            if (1..=64).contains(&tag_chars) && !tag.chars().all(|c| c == '_') {
                push_unique(&mut out.hashtags, &mut seen_hashtags, tag);
                i = j;
                continue;
            }
        }

        // 4. c/<slug> community references
        if (ch == 'c' || ch == 'C')
            && is_community_boundary_prev(prev)
            && i + 1 < len
            && chars[i + 1].1 == '/'
        {
            let mut j = i + 2;
            let mut slug = String::new();
            while j < len {
                let c = chars[j].1;
                if c.is_ascii_alphanumeric() || c == '_' {
                    slug.push(c.to_ascii_lowercase());
                    j += 1;
                } else {
                    break;
                }
            }
            if (2..=32).contains(&slug.len()) {
                push_unique(&mut out.communities, &mut seen_communities, slug);
                i = j;
                continue;
            }
        }

        i += 1;
    }

    out
}

/// Structured query operators parsed from community search input.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedSearchQuery {
    pub free_text: String,
    pub community: Option<String>,
    pub author: Option<String>,
    pub flair: Option<String>,
    pub actor_kind: Option<ActorKind>,
    pub has_media: Option<bool>,
    pub has_poll: Option<bool>,
    pub has_link: Option<bool>,
}

fn tokenize_search_input(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in raw.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
            current.push(ch);
        } else if ch.is_whitespace() && !in_quotes {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Parses inline search operators (`c:`, `author:`/`from:`, `flair:`, `kind:`, `has:`) cleanly.
pub fn parse_search_operators(raw: &str) -> ParsedSearchQuery {
    let mut parsed = ParsedSearchQuery::default();
    let mut free_parts = Vec::new();

    for tok in tokenize_search_input(raw) {
        if let Some((key_raw, val_raw)) = tok.split_once(':') {
            let key = key_raw.to_ascii_lowercase();
            let val = val_raw.trim_matches('"').trim();
            if !val.is_empty() {
                match key.as_str() {
                    "c" | "r" | "community" => {
                        let lower = val.to_ascii_lowercase();
                        let slug = lower
                            .strip_prefix("c/")
                            .or_else(|| lower.strip_prefix("r/"))
                            .unwrap_or(&lower)
                            .to_string();
                        if !slug.is_empty() {
                            parsed.community = Some(slug);
                            continue;
                        }
                    }
                    "author" | "from" => {
                        let h = val.trim_start_matches('@').to_ascii_lowercase();
                        if !h.is_empty() {
                            parsed.author = Some(h);
                            continue;
                        }
                    }
                    "flair" => {
                        parsed.flair = Some(val.to_string());
                        continue;
                    }
                    "kind" | "actor" => {
                        if let Some(kind) = ActorKind::parse(val) {
                            parsed.actor_kind = Some(kind);
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
                            parsed.has_link = Some(true);
                            continue;
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        free_parts.push(tok);
    }

    parsed.free_text = free_parts.join(" ");
    parsed
}

/// Applies or updates a voter's poll choices, maintaining exact `all_votes`, `human_votes`, and `bot_votes`.
pub fn apply_poll_vote(
    poll: &mut PollDto,
    voter_kind: ActorKind,
    option_ids: &[String],
    prev_option_ids: &[String],
    now_ms: i64,
) -> Result<()> {
    if poll.is_closed || now_ms >= poll.expires_at_ms {
        poll.is_closed = true;
        return Err(TelegramError::Validation("poll is closed".to_string()));
    }
    if option_ids.is_empty() {
        return Err(TelegramError::Validation(
            "at least one poll option must be selected".to_string(),
        ));
    }
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for id in option_ids {
        if seen.insert(id.clone()) {
            deduped.push(id.clone());
        }
    }
    if !poll.multi_select && deduped.len() > 1 {
        return Err(TelegramError::Validation(
            "poll does not allow multiple selections".to_string(),
        ));
    }
    let valid_ids: HashSet<&str> = poll.options.iter().map(|o| o.option_id.as_str()).collect();
    for id in &deduped {
        if !valid_ids.contains(id.as_str()) {
            return Err(TelegramError::Validation(format!(
                "unknown poll option_id '{id}'"
            )));
        }
    }

    let mut deduped_prev = Vec::new();
    let mut seen_prev = HashSet::new();
    for id in prev_option_ids {
        if seen_prev.insert(id.clone()) {
            deduped_prev.push(id.clone());
        }
    }

    let is_human = voter_kind == ActorKind::Human;
    for prev_id in &deduped_prev {
        if let Some(opt) = poll.options.iter_mut().find(|o| &o.option_id == prev_id) {
            opt.all_votes = opt.all_votes.saturating_sub(1);
            if is_human {
                opt.human_votes = opt.human_votes.saturating_sub(1);
            } else {
                opt.bot_votes = opt.bot_votes.saturating_sub(1);
            }
        }
    }
    for new_id in &deduped {
        if let Some(opt) = poll.options.iter_mut().find(|o| &o.option_id == new_id) {
            opt.all_votes = opt.all_votes.saturating_add(1);
            if is_human {
                opt.human_votes = opt.human_votes.saturating_add(1);
            } else {
                opt.bot_votes = opt.bot_votes.saturating_add(1);
            }
        }
    }

    poll.total_votes = poll
        .options
        .iter()
        .fold(0u64, |acc, o| acc.saturating_add(o.all_votes));
    poll.total_human_votes = poll
        .options
        .iter()
        .fold(0u64, |acc, o| acc.saturating_add(o.human_votes));
    poll.total_bot_votes = poll
        .options
        .iter()
        .fold(0u64, |acc, o| acc.saturating_add(o.bot_votes));
    poll.viewer_voted_option_ids = deduped;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_telegram_proto::community_contracts::{CommunityPollOptionDto, CreatePollInputDto};

    #[test]
    fn entity_extraction_handles_mentions_emails_cjk_communities_and_urls() {
        let text = "Hey @Ferris_Bot and @alex_chen! Email me at user@example.com (not a mention). Check c/Rust and c/AI_Agents for #RustLang #推荐算法 at https://example.org/docs/guide. Also @ferris_bot again!";
        let ent = extract_entities(text);
        assert_eq!(ent.mentions, vec!["ferris_bot", "alex_chen"]);
        assert_eq!(ent.hashtags, vec!["rustlang", "推荐算法"]);
        assert_eq!(ent.communities, vec!["rust", "ai_agents"]);
        assert_eq!(ent.urls, vec!["https://example.org/docs/guide"]);
    }

    #[test]
    fn search_operator_parser_handles_quoted_and_mixed_operators() {
        let parsed = parse_search_operators(
            r#"c:Rust author:@Ferris_Bot flair:"Deep Dive" kind:bot has:poll async lifetimes"#,
        );
        assert_eq!(parsed.community.as_deref(), Some("rust"));
        assert_eq!(parsed.author.as_deref(), Some("ferris_bot"));
        assert_eq!(parsed.flair.as_deref(), Some("Deep Dive"));
        assert_eq!(parsed.actor_kind, Some(ActorKind::Bot));
        assert_eq!(parsed.has_poll, Some(true));
        assert_eq!(parsed.free_text, "async lifetimes");
    }

    #[test]
    fn poll_vote_tracks_human_and_bot_counts_and_enforces_expiry() {
        let now = 1_760_000_000_000;
        let mut poll = PollDto {
            poll_id: "p1".into(),
            question: "Which runtime?".into(),
            options: vec![
                CommunityPollOptionDto {
                    option_id: "o0".into(),
                    text: "Tokio".into(),
                    all_votes: 0,
                    human_votes: 0,
                    bot_votes: 0,
                },
                CommunityPollOptionDto {
                    option_id: "o1".into(),
                    text: "Smol".into(),
                    all_votes: 0,
                    human_votes: 0,
                    bot_votes: 0,
                },
            ],
            multi_select: false,
            expires_at_ms: now + 60_000,
            is_closed: false,
            total_votes: 0,
            total_human_votes: 0,
            total_bot_votes: 0,
            viewer_voted_option_ids: vec![],
        };

        apply_poll_vote(&mut poll, ActorKind::Human, &["o0".into()], &[], now).unwrap();
        apply_poll_vote(&mut poll, ActorKind::Bot, &["o0".into()], &[], now).unwrap();
        assert_eq!(poll.options[0].all_votes, 2);
        assert_eq!(poll.options[0].human_votes, 1);
        assert_eq!(poll.options[0].bot_votes, 1);

        // Human switches vote from o0 to o1
        apply_poll_vote(&mut poll, ActorKind::Human, &["o1".into()], &["o0".into()], now).unwrap();
        assert_eq!(poll.options[0].human_votes, 0);
        assert_eq!(poll.options[0].bot_votes, 1);
        assert_eq!(poll.options[1].human_votes, 1);
        assert_eq!(poll.total_votes, 2);

        // Multi-select rejected on single-select poll
        assert!(apply_poll_vote(&mut poll, ActorKind::Human, &["o0".into(), "o1".into()], &[], now).is_err());

        // Expired poll rejected
        assert!(apply_poll_vote(&mut poll, ActorKind::Bot, &["o1".into()], &[], now + 60_001).is_err());
        assert!(poll.is_closed);
    }

    #[test]
    fn post_validation_rejects_invalid_poll_and_accepts_valid_poll() {
        let now = 1_760_000_000_000;
        let bad = CreatePostRequest {
            format: PostFormat::Poll,
            poll: Some(CreatePollInputDto {
                question: "Dup?".into(),
                options: vec!["Yes".into(), "yes".into()],
                multi_select: false,
                duration_secs: 3600,
            }),
            ..Default::default()
        };
        assert!(validate_create_post_request(&bad, now).is_err());

        let good = CreatePostRequest {
            community_slug: Some("c/rust".into()),
            format: PostFormat::Poll,
            poll: Some(CreatePollInputDto {
                question: "Async?".into(),
                options: vec!["Yes".into(), "No".into()],
                multi_select: false,
                duration_secs: 3600,
            }),
            ..Default::default()
        };
        assert!(validate_create_post_request(&good, now).is_ok());
    }

    #[test]
    fn entity_extraction_cjk_urls_and_rfc_email_boundaries() {
        let cjk_text =
            "请看文档（https://rust-lang.org/docs）。还有https://example.org/test，太棒了！";
        let ent = extract_entities(cjk_text);
        assert_eq!(
            ent.urls,
            vec![
                "https://rust-lang.org/docs".to_string(),
                "https://example.org/test".to_string()
            ]
        );

        let email_text =
            "Contact weird+@example.com or dash-@example.com, but ping @real_user and (@bot_two)!";
        let ent2 = extract_entities(email_text);
        assert_eq!(ent2.mentions, vec!["real_user", "bot_two"]);

        let parsed = parse_search_operators("c:C/Rust r:R/Systems has:link");
        assert_eq!(parsed.community.as_deref(), Some("systems"));
        assert_eq!(parsed.has_link, Some(true));
        let parsed_c = parse_search_operators("c:C/Rust");
        assert_eq!(parsed_c.community.as_deref(), Some("rust"));
    }

    #[test]
    fn poll_vote_dedups_prev_option_ids_and_validation_rejects_deleted_and_oversized() {
        let now = 1_760_000_000_000;
        let mut poll = PollDto {
            poll_id: "p_dedup".into(),
            question: "Choice?".into(),
            options: vec![
                CommunityPollOptionDto {
                    option_id: "o1".into(),
                    text: "One".into(),
                    all_votes: 2,
                    human_votes: 2,
                    bot_votes: 0,
                },
                CommunityPollOptionDto {
                    option_id: "o2".into(),
                    text: "Two".into(),
                    all_votes: 0,
                    human_votes: 0,
                    bot_votes: 0,
                },
            ],
            multi_select: false,
            expires_at_ms: now + 60_000,
            is_closed: false,
            total_votes: 2,
            total_human_votes: 2,
            total_bot_votes: 0,
            viewer_voted_option_ids: vec![],
        };
        // Passing duplicate prev_option_ids must only decrement o1 once (2 -> 1, NOT 2 -> 0)
        apply_poll_vote(
            &mut poll,
            ActorKind::Human,
            &["o2".into()],
            &["o1".into(), "o1".into()],
            now,
        )
        .unwrap();
        assert_eq!(poll.options[0].all_votes, 1);
        assert_eq!(poll.options[0].human_votes, 1);
        assert_eq!(poll.options[1].all_votes, 1);
        assert_eq!(poll.total_votes, 2);

        // PostStatus::Deleted must be rejected in validate_create_post_request
        let del_req = CreatePostRequest {
            format: PostFormat::Micro,
            status: PostStatus::Deleted,
            body: "hello".into(),
            ..Default::default()
        };
        assert!(validate_create_post_request(&del_req, now).is_err());

        // Oversized body must be rejected across ALL formats (e.g., Link, Quote)
        let huge_body = format!("https://example.org {}", "a".repeat(MAX_ARTICLE_BODY_CHARS + 10));
        let huge_link = CreatePostRequest {
            format: PostFormat::Link,
            body: huge_body.clone(),
            ..Default::default()
        };
        assert!(validate_create_post_request(&huge_link, now).is_err());

        let huge_quote = CreatePostRequest {
            format: PostFormat::Quote,
            quoted_post_id: Some("p_orig".into()),
            body: huge_body,
            ..Default::default()
        };
        assert!(validate_create_post_request(&huge_quote, now).is_err());
    }

    #[test]
    fn poll_vote_saturates_u64_max_totals_without_overflow() {
        let now = 1_760_000_000_000;
        let mut poll = PollDto {
            poll_id: "p_sat".into(),
            question: "Scale?".into(),
            options: vec![
                CommunityPollOptionDto {
                    option_id: "o1".into(),
                    text: "A".into(),
                    all_votes: u64::MAX - 1,
                    human_votes: u64::MAX - 1,
                    bot_votes: u64::MAX - 1,
                },
                CommunityPollOptionDto {
                    option_id: "o2".into(),
                    text: "B".into(),
                    all_votes: 10,
                    human_votes: 10,
                    bot_votes: 10,
                },
            ],
            multi_select: false,
            expires_at_ms: now + 60_000,
            is_closed: false,
            total_votes: 0,
            total_human_votes: 0,
            total_bot_votes: 0,
            viewer_voted_option_ids: vec![],
        };
        apply_poll_vote(&mut poll, ActorKind::Human, &["o2".into()], &[], now).unwrap();
        assert_eq!(poll.total_votes, u64::MAX);
        assert_eq!(poll.total_human_votes, u64::MAX);
        assert_eq!(poll.total_bot_votes, u64::MAX);
    }
}
