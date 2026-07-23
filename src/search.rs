use crate::globfilter::GlobFilter;
use crate::models::SearchResult;
use crate::store::{FileContent, RepoMap};
use regex::RegexBuilder;
use std::collections::HashMap;
use std::path::Path;

/// A repo's files worth actually scanning for a given query — narrowed by the trigram index
/// (literal queries) or the full working set (regex queries, or literal queries too short to
/// have trigrams).
pub struct RepoSnapshot {
    pub id: String,
    pub name: String,
    pub version: String,
    pub org: String,
    pub branch: String,
    pub candidates: Vec<(String, FileContent)>,
}

/// Selects, per matching repo, which files are worth scanning for `query_lower`. Runs under
/// a single read-lock acquisition — cheap, since resolving each path only clones a `Bytes`
/// (refcounted) or an `Arc<Shard>` plus a path string; the actual line-by-line scan (including
/// the mmap slice lookup) happens later, off the async runtime.
pub async fn snapshot_candidates(
    repos: &RepoMap,
    allowed_repo_ids: Option<&[String]>,
    filter_repo_ids: Option<&[String]>,
    filter_orgs: Option<&[String]>,
    filter_branches: Option<&[String]>,
    query_lower: &[u8],
    regex: bool,
) -> Vec<RepoSnapshot> {
    let repos = repos.read().await;
    let mut snapshots = Vec::with_capacity(repos.len());

    for (repo_id, repo) in repos.iter() {
        if allowed_repo_ids.is_some_and(|ids| !ids.iter().any(|id| id.eq_ignore_ascii_case(repo_id))) {
            continue;
        }
        if filter_repo_ids.is_some_and(|ids| !ids.iter().any(|id| id.eq_ignore_ascii_case(repo_id))) {
            continue;
        }
        if filter_orgs.is_some_and(|orgs| !orgs.iter().any(|org| org.eq_ignore_ascii_case(&repo.org))) {
            continue;
        }
        if filter_branches.is_some_and(|branches| !branches.iter().any(|branch| branch.eq_ignore_ascii_case(&repo.branch))) {
            continue;
        }

        // Regex queries and queries too short to have trigrams (<3 chars) fall back to a
        // full scan of the repo's working set.
        let candidate_ids: Option<Vec<usize>> = if regex {
            None
        } else {
            repo.index
                .as_ref()
                .and_then(|index| index.candidates(query_lower))
                .map(|ids| ids.into_iter().map(|id| id as usize).collect())
        };
        // `candidate_ids` (when present) is already sorted ascending by `TrigramIndex::candidates`.

        let candidates: Vec<(String, FileContent)> = match candidate_ids {
            Some(ids) => ids
                .into_iter()
                .filter_map(|id| repo.file_order.get(id))
                .filter_map(|path| repo.get_content(path).map(|content| (path.clone(), content)))
                .collect(),
            None => repo
                .file_order
                .iter()
                .filter_map(|path| repo.get_content(path).map(|content| (path.clone(), content)))
                .collect(),
        };

        snapshots.push(RepoSnapshot {
            id: repo_id.clone(),
            name: repo.name.clone(),
            version: repo.version.clone(),
            org: repo.org.clone(),
            branch: repo.branch.clone(),
            candidates,
        });
    }

    snapshots
}

/// Line-by-line match over a repo's candidate files. CPU-bound — callers should run this via
/// `spawn_blocking`.
pub fn scan_repo(
    snapshot: &RepoSnapshot,
    query: &str,
    regex: bool,
    case_insensitive: bool,
    file_types: Option<&[String]>,
    path_prefix: Option<&str>,
    glob_filter: Option<&GlobFilter>,
) -> (Vec<SearchResult>, HashMap<(String, String), usize>) {
    let mut results = Vec::new();
    let mut facets: HashMap<(String, String), usize> = HashMap::new();

    let query_lower = query.to_lowercase();
    let find_in_line: Box<dyn Fn(&str) -> Option<usize>> = if regex {
        match RegexBuilder::new(query).case_insensitive(case_insensitive).build() {
            Ok(re) => Box::new(move |line: &str| re.find(line).map(|m| m.start())),
            Err(_) => return (results, facets),
        }
    } else if case_insensitive {
        let needle = query_lower.clone();
        Box::new(move |line: &str| line.to_lowercase().find(&needle))
    } else {
        let needle = query.to_string();
        Box::new(move |line: &str| line.find(&needle))
    };

    for (path, content) in &snapshot.candidates {
        if let Some(prefix) = path_prefix {
            if !path.starts_with(prefix) {
                continue;
            }
        }

        if let Some(filter) = glob_filter {
            if !filter.matches(path) {
                continue;
            }
        }

        let ext = Path::new(path).extension().and_then(|e| e.to_str());
        if let Some(types) = file_types {
            let matches_type = ext.is_some_and(|ext| types.iter().any(|t| t.trim_start_matches('.').eq_ignore_ascii_case(ext)));
            if !matches_type {
                continue;
            }
        }

        let Some(bytes) = content.as_bytes() else {
            continue; // path no longer resolves (e.g. shard file went missing), skip
        };
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue; // binary/non-utf8, skip
        };

        // Collected once per file so context lines are cheap slices rather than a second pass.
        let lines: Vec<&str> = text.lines().collect();
        const CONTEXT_LINES: usize = 2;

        for (line_idx, &line) in lines.iter().enumerate() {
            let Some(byte_col) = find_in_line(line) else {
                continue;
            };

            let path_bonus = if path.to_lowercase().contains(&query_lower) { 0.3 } else { 0.0 };
            let score = (0.5_f64 + path_bonus).min(1.0);

            let before_start = line_idx.saturating_sub(CONTEXT_LINES);
            let after_end = (line_idx + 1 + CONTEXT_LINES).min(lines.len());

            results.push(SearchResult {
                repo_id: snapshot.id.clone(),
                repo_name: snapshot.name.clone(),
                path: path.clone(),
                line: (line_idx + 1) as u64,
                column: (byte_col + 1) as u64,
                snippet: line.to_string(),
                context_before: lines[before_start..line_idx].iter().map(|l| l.to_string()).collect(),
                context_after: lines[(line_idx + 1)..after_end].iter().map(|l| l.to_string()).collect(),
                score,
                blob_sha: snapshot.version.clone(),
                org: snapshot.org.clone(),
                branch: snapshot.branch.clone(),
            });

            *facets.entry((snapshot.id.clone(), "repo".to_string())).or_insert(0) += 1;
            if let Some(ext) = ext {
                *facets.entry((format!(".{ext}"), "file_type".to_string())).or_insert(0) += 1;
            }
            if !snapshot.org.is_empty() {
                *facets.entry((snapshot.org.clone(), "org".to_string())).or_insert(0) += 1;
            }
            if !snapshot.branch.is_empty() {
                *facets.entry((snapshot.branch.clone(), "branch".to_string())).or_insert(0) += 1;
            }
        }
    }

    (results, facets)
}
