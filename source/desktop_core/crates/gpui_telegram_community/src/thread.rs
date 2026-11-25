//! Nested Reddit comment forest builder (all 6 sorts, stickied, collapse, load-more) and X thread/quote assembler.

use crate::recommender::scoring::{reddit_controversial, wilson_lower_bound};
use gpui_telegram_proto::community_contracts::{
    CommentSort, CommunityCommentDto, CommunityPostDto,
};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// Constructs a materialized tree path for a comment.
pub fn make_comment_path(parent_path: Option<&str>, comment_id: &str) -> String {
    match parent_path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => format!("{p}/{comment_id}"),
        None => comment_id.to_string(),
    }
}

/// Computes comment nesting depth from its materialized path (`0` for top-level).
pub fn comment_depth_from_path(path: &str) -> u32 {
    let trimmed = path.trim().trim_matches('/');
    if trimmed.is_empty() {
        return 0;
    }
    trimmed.split('/').count().saturating_sub(1) as u32
}

/// Options controlling tree sorting, collapse, depth cap, and sibling pagination.
#[derive(Debug, Clone, Default)]
pub struct CommentTreeOptions {
    pub sort: CommentSort,
    pub max_depth: Option<u32>,
    pub max_children_per_node: Option<usize>,
    pub collapsed_ids: HashSet<String>,
}

/// Builds deterministic, cycle-safe, collapsible nested Reddit comment forests.
pub struct CommentTreeBuilder;

impl CommentTreeBuilder {
    /// Builds a nested comment forest from flat comment rows sorted by `sort` with unlimited depth.
    pub fn build(flat: Vec<CommunityCommentDto>, sort: CommentSort) -> Vec<CommunityCommentDto> {
        let opts = CommentTreeOptions {
            sort,
            max_depth: None,
            max_children_per_node: None,
            collapsed_ids: HashSet::new(),
        };
        Self::build_with_options(flat, &opts)
    }

    /// Builds a nested comment forest applying sort, collapse, depth limit, and per-node child pagination.
    pub fn build_with_options(
        flat: Vec<CommunityCommentDto>,
        opts: &CommentTreeOptions,
    ) -> Vec<CommunityCommentDto> {
        if flat.is_empty() {
            return Vec::new();
        }

        let mut nodes: HashMap<String, CommunityCommentDto> = HashMap::with_capacity(flat.len());
        let mut insertion_order: Vec<String> = Vec::with_capacity(flat.len());
        for mut c in flat {
            c.children.clear();
            c.more_children_count = 0;
            let id = c.comment_id.clone();
            if !nodes.contains_key(&id) {
                insertion_order.push(id.clone());
            }
            nodes.insert(id, c);
        }

        // Break any cycles safely and promote orphans whose parent is not present in `flat`.
        let mut parent_map: HashMap<String, Option<String>> = HashMap::with_capacity(nodes.len());
        for id in &insertion_order {
            let candidate_parent = nodes[id]
                .parent_comment_id
                .as_ref()
                .filter(|pid| *pid != id && nodes.contains_key(*pid))
                .cloned();
            parent_map.insert(id.clone(), candidate_parent);
        }

        // O(N) cycle detection via 3-state coloring (0 = unvisited, 1 = visiting, 2 = verified acyclic).
        let mut visit_state: HashMap<String, u8> = HashMap::with_capacity(nodes.len());
        for id in &insertion_order {
            if visit_state.get(id).copied().unwrap_or(0) == 2 {
                continue;
            }
            let mut trail: Vec<String> = Vec::new();
            let mut cur = Some(id.clone());
            while let Some(curr_id) = cur {
                match visit_state.get(&curr_id).copied().unwrap_or(0) {
                    2 => break,
                    1 => {
                        // Detach the edge from the last node in `trail` that closed the cycle.
                        if let Some(last) = trail.last() {
                            parent_map.insert(last.clone(), None);
                        }
                        break;
                    }
                    _ => {
                        visit_state.insert(curr_id.clone(), 1);
                        trail.push(curr_id.clone());
                        cur = parent_map.get(&curr_id).and_then(|p| p.clone());
                    }
                }
            }
            for node_id in trail {
                visit_state.insert(node_id, 2);
            }
        }

        // Build adjacency lists.
        let mut children_map: HashMap<String, Vec<String>> = HashMap::new();
        let mut roots: Vec<String> = Vec::new();
        for id in &insertion_order {
            match parent_map.get(id).and_then(|p| p.clone()) {
                Some(pid) => children_map.entry(pid).or_default().push(id.clone()),
                None => roots.push(id.clone()),
            }
        }

        // Precompute `has_op_reply` iteratively bottom-up for `CommentSort::Qa` (zero recursion).
        let mut preorder: Vec<String> = Vec::with_capacity(nodes.len());
        let mut dfs_stack: Vec<String> = roots.iter().rev().cloned().collect();
        while let Some(id) = dfs_stack.pop() {
            preorder.push(id.clone());
            if let Some(kids) = children_map.get(&id) {
                for kid in kids.iter().rev() {
                    dfs_stack.push(kid.clone());
                }
            }
        }
        let mut has_op_reply: HashMap<String, bool> = HashMap::with_capacity(nodes.len());
        for id in preorder.iter().rev() {
            let mut any_op = false;
            if let Some(kids) = children_map.get(id) {
                for kid in kids {
                    let kid_is_op = nodes.get(kid).is_some_and(|n| n.is_op);
                    let desc_is_op = has_op_reply.get(kid).copied().unwrap_or(false);
                    if kid_is_op || desc_is_op {
                        any_op = true;
                        break;
                    }
                }
            }
            has_op_reply.insert(id.clone(), any_op);
        }

        // Deterministically sort root and child lists before consuming `nodes`.
        roots.sort_by(|a, b| compare_comments(&nodes[a], &nodes[b], &has_op_reply, opts.sort));
        for kid_ids in children_map.values_mut() {
            kid_ids.sort_by(|a, b| compare_comments(&nodes[a], &nodes[b], &has_op_reply, opts.sort));
        }

        // Iterative top-down pass to assign consistent `depth`, `path`, and topological order.
        let mut depth_map: HashMap<String, u32> = HashMap::with_capacity(nodes.len());
        let mut path_map: HashMap<String, String> = HashMap::with_capacity(nodes.len());
        let mut topo_order: Vec<String> = Vec::with_capacity(nodes.len());
        let mut walk_stack: Vec<(String, u32, Option<String>)> = roots
            .iter()
            .rev()
            .map(|r| (r.clone(), 0u32, None))
            .collect();
        while let Some((id, depth, parent_path)) = walk_stack.pop() {
            let path = make_comment_path(parent_path.as_deref(), &id);
            depth_map.insert(id.clone(), depth);
            path_map.insert(id.clone(), path.clone());
            topo_order.push(id.clone());
            if let Some(kids) = children_map.get(&id) {
                for kid in kids.iter().rev() {
                    walk_stack.push((kid.clone(), depth.saturating_add(1), Some(path.clone())));
                }
            }
        }

        // Iterative bottom-up assembly applying collapse, depth cap, and child pagination.
        let mut assembled: HashMap<String, (CommunityCommentDto, u32)> =
            HashMap::with_capacity(nodes.len());
        for id in topo_order.iter().rev() {
            let Some(mut node) = nodes.remove(id) else {
                continue;
            };
            let depth = depth_map.get(id).copied().unwrap_or(0);
            node.depth = depth;
            node.path = path_map.remove(id).unwrap_or_else(|| node.comment_id.clone());

            let mut assembled_kids = Vec::new();
            let mut total_descendants: u32 = 0;
            if let Some(kid_ids) = children_map.get(id) {
                assembled_kids.reserve(kid_ids.len());
                for kid_id in kid_ids {
                    if let Some((kid_node, kid_subtree_size)) = assembled.remove(kid_id) {
                        total_descendants = total_descendants
                            .saturating_add(1)
                            .saturating_add(kid_subtree_size);
                        assembled_kids.push((kid_node, kid_subtree_size));
                    }
                }
            }

            let is_collapsed = opts.collapsed_ids.contains(&node.comment_id);
            let at_depth_cap = opts.max_depth.is_some_and(|max_d| depth >= max_d);
            if is_collapsed || at_depth_cap {
                node.more_children_count = total_descendants;
                node.children = Vec::new();
            } else if let Some(max_kids) = opts.max_children_per_node {
                if assembled_kids.len() > max_kids {
                    let mut pruned: u32 = 0;
                    for (_, sub_sz) in assembled_kids.drain(max_kids..) {
                        pruned = pruned.saturating_add(1).saturating_add(sub_sz);
                    }
                    node.more_children_count = pruned;
                }
                node.children = assembled_kids.into_iter().map(|(k, _)| k).collect();
            } else {
                node.children = assembled_kids.into_iter().map(|(k, _)| k).collect();
            }

            assembled.insert(id.clone(), (node, total_descendants));
        }

        roots
            .into_iter()
            .filter_map(|r| assembled.remove(&r).map(|(node, _)| node))
            .collect()
    }
}

fn cmp_f64_desc(a: f64, b: f64) -> Ordering {
    let a = if a.is_finite() { a } else { f64::NEG_INFINITY };
    let b = if b.is_finite() { b } else { f64::NEG_INFINITY };
    b.total_cmp(&a)
}

fn compare_comments(
    a: &CommunityCommentDto,
    b: &CommunityCommentDto,
    has_op_reply: &HashMap<String, bool>,
    sort: CommentSort,
) -> Ordering {
    // Stickied comments always pin to top of their sibling level.
    b.is_stickied
        .cmp(&a.is_stickied)
        .then_with(|| match sort {
            CommentSort::Best => cmp_f64_desc(
                wilson_lower_bound(a.upvotes, a.downvotes, 1.96),
                wilson_lower_bound(b.upvotes, b.downvotes, 1.96),
            ),
            CommentSort::Top => b.score.cmp(&a.score),
            CommentSort::New => b.created_at_ms.cmp(&a.created_at_ms),
            CommentSort::Controversial => cmp_f64_desc(
                reddit_controversial(a.upvotes, a.downvotes),
                reddit_controversial(b.upvotes, b.downvotes),
            ),
            CommentSort::Old => a.created_at_ms.cmp(&b.created_at_ms),
            CommentSort::Qa => {
                let a_has = has_op_reply.get(&a.comment_id).copied().unwrap_or(false);
                let b_has = has_op_reply.get(&b.comment_id).copied().unwrap_or(false);
                b_has
                    .cmp(&a_has)
                    .then_with(|| b.is_op.cmp(&a.is_op))
                    .then_with(|| {
                        cmp_f64_desc(
                            wilson_lower_bound(a.upvotes, a.downvotes, 1.96),
                            wilson_lower_bound(b.upvotes, b.downvotes, 1.96),
                        )
                    })
            }
        })
        .then_with(|| b.is_mod_distinguished.cmp(&a.is_mod_distinguished))
        .then_with(|| a.created_at_ms.cmp(&b.created_at_ms))
        .then_with(|| a.comment_id.cmp(&b.comment_id))
}

/// Assembles X-style linear multi-post threads (`1/N`) and cycle-safe quote chains.
pub struct ThreadAssembler;

impl ThreadAssembler {
    /// Collects all thread segments belonging to the same author & thread root, sorted `(thread_index, created_at_ms, post_id)`.
    pub fn build_linear_thread(
        target: &CommunityPostDto,
        pool: &[CommunityPostDto],
    ) -> Vec<CommunityPostDto> {
        let root_id = target
            .thread_root_id
            .as_deref()
            .unwrap_or(target.post_id.as_str());
        let mut chain: Vec<CommunityPostDto> = pool
            .iter()
            .filter(|p| {
                p.author_actor_id == target.author_actor_id
                    && (p.post_id == root_id || p.thread_root_id.as_deref() == Some(root_id))
            })
            .cloned()
            .collect();
        if !chain.iter().any(|p| p.post_id == target.post_id) {
            chain.push(target.clone());
        }
        chain.sort_by(|a, b| {
            a.thread_index
                .cmp(&b.thread_index)
                .then_with(|| a.created_at_ms.cmp(&b.created_at_ms))
                .then_with(|| a.post_id.cmp(&b.post_id))
        });
        chain.dedup_by(|a, b| a.post_id == b.post_id);
        chain
    }

    /// Walks `quoted_post_id` iteratively with cycle protection up to `max_depth` ancestors.
    pub fn build_quote_chain(
        post: &CommunityPostDto,
        pool: &[CommunityPostDto],
        max_depth: usize,
    ) -> Vec<CommunityPostDto> {
        let lookup: HashMap<&str, &CommunityPostDto> =
            pool.iter().map(|p| (p.post_id.as_str(), p)).collect();
        let mut chain = Vec::new();
        let mut visited = HashSet::new();
        visited.insert(post.post_id.clone());

        let mut next_id = post.quoted_post_id.as_deref();
        while let Some(qid) = next_id {
            if chain.len() >= max_depth || !visited.insert(qid.to_string()) {
                break;
            }
            if let Some(&quoted) = lookup.get(qid) {
                chain.push(quoted.clone());
                next_id = quoted.quoted_post_id.as_deref();
            } else {
                break;
            }
        }
        chain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_telegram_proto::community_contracts::{ActorKind, PostFormat, PostStatus, VerifiedTier};

    fn mk_comment(
        id: &str,
        parent: Option<&str>,
        up: i64,
        down: i64,
        ts: i64,
        stickied: bool,
        is_op: bool,
    ) -> CommunityCommentDto {
        CommunityCommentDto {
            comment_id: id.into(),
            post_id: "p1".into(),
            parent_comment_id: parent.map(str::to_string),
            depth: 0,
            path: String::new(),
            author_actor_id: if is_op { "u:1".into() } else { "u:2".into() },
            author_handle: if is_op { "op".into() } else { "guest".into() },
            author_display_name: "User".into(),
            author_kind: ActorKind::Human,
            author_verified_tier: VerifiedTier::None,
            author_bot_badge: None,
            author_avatar_shape: "classic".into(),
            author_avatar_color: "blue".into(),
            is_op,
            body: format!("body {id}"),
            media: vec![],
            upvotes: up,
            downvotes: down,
            score: up - down,
            likes_count: 0,
            awards: vec![],
            viewer_vote: 0,
            viewer_liked: false,
            is_stickied: stickied,
            is_mod_distinguished: stickied,
            is_removed: false,
            is_deleted: false,
            children: vec![],
            more_children_count: 0,
            created_at_ms: ts,
            updated_at_ms: ts,
        }
    }

    fn mk_post(id: &str, root: Option<&str>, idx: u32, quoted: Option<&str>) -> CommunityPostDto {
        CommunityPostDto {
            post_id: id.into(),
            community_id: None,
            community_slug: Some("rust".into()),
            author_actor_id: "u:1".into(),
            author_handle: "alex".into(),
            author_display_name: "Alex".into(),
            author_kind: ActorKind::Human,
            author_verified_tier: VerifiedTier::None,
            author_bot_badge: None,
            author_avatar_media_id: None,
            author_avatar_shape: "classic".into(),
            author_avatar_color: "blue".into(),
            co_author_actor_id: None,
            format: PostFormat::Micro,
            status: PostStatus::Published,
            title: None,
            body: format!("post {id}"),
            flair: None,
            tags: vec![],
            mentions: vec![],
            media: vec![],
            link_preview: None,
            poll: None,
            quoted_post_id: quoted.map(str::to_string),
            quoted_post: None,
            reposted_post_id: None,
            reposted_post: None,
            crosspost_of_id: None,
            crosspost_origin: None,
            thread_root_id: root.map(str::to_string),
            parent_post_id: None,
            thread_index: idx,
            thread_total: 4,
            is_nsfw: false,
            is_spoiler: false,
            is_pinned: false,
            is_locked: false,
            is_removed: false,
            is_mod_distinguished: false,
            upvotes: 1,
            downvotes: 0,
            score: 1,
            likes_count: 0,
            reposts_count: 0,
            quotes_count: 0,
            comments_count: 0,
            bookmarks_count: 0,
            views_count: 0,
            awards: vec![],
            viewer_vote: 0,
            viewer_liked: false,
            viewer_reposted: false,
            viewer_bookmarked: false,
            viewer_bookmark_folder: None,
            edit_history: vec![],
            scheduled_at_ms: None,
            created_at_ms: 1000 + i64::from(idx),
            updated_at_ms: 1000 + i64::from(idx),
        }
    }

    #[test]
    fn comment_tree_sorts_all_six_modes_pins_stickied_and_handles_cycles_orphans_collapse() {
        let flat = vec![
            mk_comment("c_sticky", None, 1, 0, 100, true, false),
            mk_comment("c_top", None, 100, 10, 200, false, false),
            mk_comment("c_contro", None, 55, 50, 300, false, false),
            mk_comment("c_new", None, 2, 0, 500, false, false),
            mk_comment("c_qa_parent", None, 5, 0, 250, false, false),
            mk_comment("c_qa_op_reply", Some("c_qa_parent"), 3, 0, 260, false, true),
            mk_comment("c_deep_1", Some("c_top"), 10, 0, 210, false, false),
            mk_comment("c_deep_2", Some("c_deep_1"), 4, 0, 220, false, false),
            mk_comment("c_orphan", Some("missing_parent"), 1, 0, 150, false, false),
        ];

        let top_tree = CommentTreeBuilder::build(flat.clone(), CommentSort::Top);
        assert_eq!(top_tree[0].comment_id, "c_sticky");
        assert_eq!(top_tree[1].comment_id, "c_top");
        assert_eq!(top_tree[1].children[0].depth, 1);
        assert_eq!(top_tree[1].children[0].children[0].depth, 2);
        assert!(top_tree.iter().any(|c| c.comment_id == "c_orphan"));

        let contro = CommentTreeBuilder::build(flat.clone(), CommentSort::Controversial);
        assert_eq!(contro[1].comment_id, "c_contro");

        let newest = CommentTreeBuilder::build(flat.clone(), CommentSort::New);
        assert_eq!(newest[1].comment_id, "c_new");

        let oldest = CommentTreeBuilder::build(flat.clone(), CommentSort::Old);
        assert_eq!(oldest[1].comment_id, "c_orphan");

        let qa = CommentTreeBuilder::build(flat.clone(), CommentSort::Qa);
        assert_eq!(qa[1].comment_id, "c_qa_parent");

        let opts = CommentTreeOptions {
            sort: CommentSort::Best,
            max_depth: Some(0),
            max_children_per_node: None,
            collapsed_ids: HashSet::new(),
        };
        let capped = CommentTreeBuilder::build_with_options(flat, &opts);
        let c_top_capped = capped.iter().find(|c| c.comment_id == "c_top").unwrap();
        assert!(c_top_capped.children.is_empty());
        assert_eq!(c_top_capped.more_children_count, 2);

        // Cyclic parent references must not panic or lose nodes.
        let cyclic = vec![
            mk_comment("a", Some("b"), 5, 0, 10, false, false),
            mk_comment("b", Some("a"), 4, 0, 20, false, false),
        ];
        let cyc_tree = CommentTreeBuilder::build(cyclic, CommentSort::Top);
        let count_nodes = cyc_tree.len() + cyc_tree.iter().map(|n| n.children.len()).sum::<usize>();
        assert_eq!(count_nodes, 2);
    }

    #[test]
    fn thread_assembler_builds_linear_thread_and_cycle_safe_quote_chain() {
        let pool = vec![
            mk_post("t3", Some("t1"), 2, None),
            mk_post("t1", Some("t1"), 0, None),
            mk_post("t4", Some("t1"), 3, None),
            mk_post("t2", Some("t1"), 1, None),
        ];
        let linear = ThreadAssembler::build_linear_thread(&pool[0], &pool);
        let ids: Vec<&str> = linear.iter().map(|p| p.post_id.as_str()).collect();
        assert_eq!(ids, vec!["t1", "t2", "t3", "t4"]);

        let q1 = mk_post("q1", None, 0, Some("q2"));
        let q2 = mk_post("q2", None, 0, Some("q1")); // cycle!
        let chain = ThreadAssembler::build_quote_chain(&q1, &[q1.clone(), q2], 8);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].post_id, "q2");
    }

    #[test]
    fn comment_tree_handles_1500_deep_chain_without_stack_overflow() {
        let depth_count: usize = 1_500;
        let mut flat = Vec::with_capacity(depth_count);
        for i in 0..depth_count {
            let parent = if i == 0 {
                None
            } else {
                Some(format!("c_{}", i - 1))
            };
            let is_op = i + 1 == depth_count;
            flat.push(mk_comment(
                &format!("c_{i}"),
                parent.as_deref(),
                10,
                1,
                1_000 + i as i64,
                false,
                is_op,
            ));
        }

        // Must assemble cleanly in Q&A mode (which propagates `has_op_reply` from depth 1499 to root!)
        let tree = CommentTreeBuilder::build(flat, CommentSort::Qa);
        assert_eq!(tree.len(), 1);

        // Walk iteratively to verify exact depth 0..1499
        let mut cur = &tree[0];
        for expected_depth in 0..depth_count {
            assert_eq!(cur.comment_id, format!("c_{expected_depth}"));
            assert_eq!(cur.depth, expected_depth as u32);
            if expected_depth + 1 < depth_count {
                assert_eq!(cur.children.len(), 1);
                cur = &cur.children[0];
            } else {
                assert!(cur.children.is_empty());
            }
        }
    }
}
