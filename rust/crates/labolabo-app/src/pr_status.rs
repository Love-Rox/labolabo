//! GitHub PR status badge (`plans` 第16波 #3): for a worktree Task whose
//! branch is known, look up whether an open/draft/merged/closed PR exists
//! for that branch via the `gh` CLI, and expose the result to `crate::
//! sidebar`'s row badge / `crate::titlebar`'s status-pill segment.
//!
//! ## Design
//!
//! - **`gh` only, resolved via `labolabo_core::tool_locator`** (same
//!   "fixed candidates -> PATH -> login shell" resolution every other
//!   external tool this app shells out to already uses). Missing/
//!   unauthenticated `gh` collapses to `None` -- no error banner, no retry
//!   affordance, exactly the same silent-degrade posture `crate::
//!   update_check`'s `curl` calls already established for this app.
//! - **No DB persistence.** A PR's state is exactly as volatile as the
//!   remote itself (comments/reviews/CI can flip draft->open->merged at any
//!   time outside this app's control), so the fetched result only ever
//!   lives in `LaboLaboApp::pr_cache` (in-memory `HashMap`) -- unlike
//!   `Task::color`/`windowBounds`/etc., there is nothing here worth
//!   surviving a restart; a fresh launch just re-fetches on first Task
//!   selection.
//! - **Throttled per Task, not polled.** `gh pr list` is a real network
//!   call to the GitHub API (unlike a local `git status`), so `app.rs`'s
//!   `LaboLaboApp::maybe_refresh_pr_status` (this module's only caller)
//!   only fires it on Task selection and on a completed Git refresh, and
//!   even then only if [`should_refresh`] says [`REFRESH_INTERVAL`] has
//!   elapsed since the last fetch for that Task -- never on a bare timer,
//!   and never on every `FileWatcher` tick (that would hammer the API on a
//!   busy repo).
//! - **`gh` invocation is trait-injected** ([`GhPrLister`]) so this
//!   module's own tests exercise [`fetch_pr_info`]'s parsing/plumbing
//!   against canned JSON, never a real `gh`/network call -- this repo's CI
//!   must never depend on live GitHub API access.

use std::path::Path;
use std::time::Duration;

use labolabo_core::{ToolLocating, ToolLocator};

/// Minimum interval between two `gh pr list` fetches for the *same* Task
/// (`plans` 第16波 #3: "同一タスクは最短120秒間隔").
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(120);

/// Wall-clock budget for one `gh pr list` invocation -- generous enough for
/// a slow connection, but still well short of anything a user would
/// perceive as a hang (this always runs on a background thread, never
/// blocking the UI either way).
const GH_TIMEOUT: Duration = Duration::from_secs(10);

/// One fetched PR's state, reduced to what the sidebar badge / tooltip /
/// titlebar pill need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo {
    pub number: u64,
    pub state: PrState,
    pub title: String,
    pub url: String,
}

/// A PR's lifecycle state, as the badge distinguishes it -- GitHub's own
/// `state`/`isDraft` fields collapsed into one enum (`state_from_fields`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Draft,
    Open,
    Merged,
    Closed,
}

/// [`PrState`] -> badge color (`crate::theme::pr`) -- see that module's doc
/// comment for the GitHub-convention rationale.
pub fn badge_color(state: PrState) -> u32 {
    match state {
        PrState::Draft => crate::theme::pr::DRAFT,
        PrState::Open => crate::theme::pr::OPEN,
        PrState::Merged => crate::theme::pr::MERGED,
        PrState::Closed => crate::theme::pr::CLOSED,
    }
}

/// `gh pr list`'s `state`/`isDraft` JSON fields -> [`PrState`]. `None` for
/// an unrecognized `state` value (shouldn't happen against the real API,
/// but this module's usual "unknown input degrades to nothing" posture --
/// same as `TaskStatus::parse`/`AgentBindings::from_json` elsewhere in this
/// codebase).
fn state_from_fields(state: &str, is_draft: bool) -> Option<PrState> {
    match state {
        "OPEN" if is_draft => Some(PrState::Draft),
        "OPEN" => Some(PrState::Open),
        "MERGED" => Some(PrState::Merged),
        "CLOSED" => Some(PrState::Closed),
        _ => None,
    }
}

/// Parses `gh pr list --json number,state,isDraft,title,url --limit 1`'s
/// stdout (a JSON array of 0 or 1 entries): `None` for an empty array (no
/// PR for this branch), malformed JSON, a non-array body, or a missing/
/// unparseable required field on the one entry -- every failure mode
/// collapses to "no badge", per this module's silent-degrade contract (see
/// module doc comment).
pub fn parse_pr_list(body: &str) -> Option<PrInfo> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let entry = value.as_array()?.first()?;
    let number = entry.get("number")?.as_u64()?;
    let state_str = entry.get("state")?.as_str()?;
    let is_draft = entry
        .get("isDraft")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let state = state_from_fields(state_str, is_draft)?;
    let title = entry
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let url = entry
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Some(PrInfo {
        number,
        state,
        title,
        url,
    })
}

/// `true` if `elapsed_since_last_fetch` (the caller's `Instant::
/// duration_since` against the cache entry's own fetch time, `None` when
/// there is no cache entry yet -- i.e. never fetched) is old enough (or
/// absent) to justify another `gh pr list` call, per [`REFRESH_INTERVAL`]'s
/// throttle. Takes an already-computed `Duration` rather than two
/// `Instant`s so it stays a plain, deterministic pure function -- callers
/// (`app::LaboLaboApp::maybe_refresh_pr_status`) do the real-clock math.
pub fn should_refresh(elapsed_since_last_fetch: Option<Duration>) -> bool {
    match elapsed_since_last_fetch {
        None => true,
        Some(elapsed) => elapsed >= REFRESH_INTERVAL,
    }
}

/// `true` if `repo_name` looks like a `owner/repo` GitHub slug (exactly one
/// `/`, non-empty on both sides) -- `Task::repo_name`'s own doc comment
/// says it's `owner/repo` "when a remote is configured, else the folder
/// name" (`labolabo_core::store::task_record`), so a Task whose repo has no
/// GitHub remote (or a non-GitHub host) never looks like a slug here. A
/// cheap pre-filter `LaboLaboApp::maybe_refresh_pr_status` uses to skip a
/// doomed-to-fail `gh pr list` process spawn entirely, rather than paying
/// for one every throttle window just to get silently `None`-filtered by
/// [`parse_pr_list`]/`gh` itself.
pub fn looks_like_github_slug(repo_name: &str) -> bool {
    match repo_name.split_once('/') {
        Some((owner, repo)) => !owner.is_empty() && !repo.is_empty() && !repo.contains('/'),
        None => false,
    }
}

/// Abstraction over "run `gh pr list` for this branch/repo, return raw
/// stdout on success" -- see module doc comment for why this is trait-
/// injected (test fakeability, no real GitHub API access from CI).
pub trait GhPrLister {
    /// `None` on any failure (`gh` not found, non-zero exit, timeout) --
    /// callers ([`fetch_pr_info`]) treat that identically to "no PR found",
    /// per this module's silent-degrade contract.
    fn list_prs(&self, branch: &str, repo: &str) -> Option<String>;
}

/// The real [`GhPrLister`]: resolves `gh` via [`ToolLocator`], then runs
/// `gh pr list --head <branch> --repo <repo> --json
/// number,state,isDraft,title,url --limit 1`.
pub struct RealGhPrLister;

impl GhPrLister for RealGhPrLister {
    fn list_prs(&self, branch: &str, repo: &str) -> Option<String> {
        let gh_path = ToolLocator.locate("gh")?;
        run_gh_pr_list(&gh_path, branch, repo)
    }
}

fn run_gh_pr_list(gh_path: &Path, branch: &str, repo: &str) -> Option<String> {
    let args = [
        "pr".to_string(),
        "list".to_string(),
        "--head".to_string(),
        branch.to_string(),
        "--repo".to_string(),
        repo.to_string(),
        "--json".to_string(),
        "number,state,isDraft,title,url".to_string(),
        "--limit".to_string(),
        "1".to_string(),
    ];
    match labolabo_core::process::run_with_timeout(gh_path, &args, None, None, GH_TIMEOUT) {
        Ok(Some(output)) if output.status == 0 => Some(output.stdout),
        _ => None,
    }
}

/// Fetches `branch`'s PR info in `repo` (`owner/repo`) via `lister`,
/// parsing its raw stdout through [`parse_pr_list`]. Blocking -- call from
/// a background thread (`cx.background_spawn`, mirrors every other
/// external-process call in this crate).
pub fn fetch_pr_info(lister: &dyn GhPrLister, branch: &str, repo: &str) -> Option<PrInfo> {
    let body = lister.list_prs(branch, repo)?;
    parse_pr_list(&body)
}

/// [`fetch_pr_info`] against the real [`RealGhPrLister`] -- `app.rs`'s
/// production entry point.
pub fn fetch_pr_info_default(branch: &str, repo: &str) -> Option<PrInfo> {
    fetch_pr_info(&RealGhPrLister, branch, repo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // MARK: - parse_pr_list

    #[test]
    fn parses_a_draft_pr() {
        let body = r#"[{"number":123,"state":"OPEN","isDraft":true,"title":"WIP: thing","url":"https://github.com/o/r/pull/123"}]"#;
        let info = parse_pr_list(body).expect("pr");
        assert_eq!(info.number, 123);
        assert_eq!(info.state, PrState::Draft);
        assert_eq!(info.title, "WIP: thing");
        assert_eq!(info.url, "https://github.com/o/r/pull/123");
    }

    #[test]
    fn parses_an_open_pr() {
        let body = r#"[{"number":5,"state":"OPEN","isDraft":false,"title":"Fix bug","url":"https://x/5"}]"#;
        assert_eq!(parse_pr_list(body).unwrap().state, PrState::Open);
    }

    #[test]
    fn parses_a_merged_pr() {
        let body = r#"[{"number":5,"state":"MERGED","isDraft":false,"title":"t","url":"u"}]"#;
        assert_eq!(parse_pr_list(body).unwrap().state, PrState::Merged);
    }

    #[test]
    fn parses_a_closed_pr() {
        let body = r#"[{"number":5,"state":"CLOSED","isDraft":false,"title":"t","url":"u"}]"#;
        assert_eq!(parse_pr_list(body).unwrap().state, PrState::Closed);
    }

    #[test]
    fn empty_array_means_no_pr_for_the_branch() {
        assert_eq!(parse_pr_list("[]"), None);
    }

    #[test]
    fn malformed_or_non_array_json_is_none() {
        assert_eq!(parse_pr_list("not json"), None);
        assert_eq!(parse_pr_list(""), None);
        assert_eq!(parse_pr_list(r#"{"number":1}"#), None);
    }

    #[test]
    fn missing_required_fields_is_none() {
        assert_eq!(parse_pr_list(r#"[{"number":1}]"#), None);
        assert_eq!(parse_pr_list(r#"[{"state":"OPEN"}]"#), None);
    }

    #[test]
    fn missing_is_draft_defaults_to_false() {
        let body = r#"[{"number":1,"state":"OPEN","title":"t","url":"u"}]"#;
        assert_eq!(parse_pr_list(body).unwrap().state, PrState::Open);
    }

    #[test]
    fn unrecognized_state_is_none() {
        let body = r#"[{"number":1,"state":"WEIRD","isDraft":false,"title":"t","url":"u"}]"#;
        assert_eq!(parse_pr_list(body), None);
    }

    // MARK: - badge_color

    #[test]
    fn badge_color_maps_every_state_to_a_distinct_theme_token() {
        assert_eq!(badge_color(PrState::Draft), crate::theme::pr::DRAFT);
        assert_eq!(badge_color(PrState::Open), crate::theme::pr::OPEN);
        assert_eq!(badge_color(PrState::Merged), crate::theme::pr::MERGED);
        assert_eq!(badge_color(PrState::Closed), crate::theme::pr::CLOSED);
        let colors = [
            badge_color(PrState::Draft),
            badge_color(PrState::Open),
            badge_color(PrState::Merged),
            badge_color(PrState::Closed),
        ];
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(colors[i], colors[j], "PR state colors must be distinct");
            }
        }
    }

    // MARK: - should_refresh

    #[test]
    fn should_refresh_true_when_never_fetched() {
        assert!(should_refresh(None));
    }

    #[test]
    fn should_refresh_false_within_the_throttle_window() {
        assert!(!should_refresh(Some(Duration::from_secs(1))));
        assert!(!should_refresh(Some(
            REFRESH_INTERVAL - Duration::from_secs(1)
        )));
    }

    #[test]
    fn should_refresh_true_once_the_window_has_elapsed() {
        assert!(should_refresh(Some(REFRESH_INTERVAL)));
        assert!(should_refresh(Some(
            REFRESH_INTERVAL + Duration::from_secs(1)
        )));
    }

    // MARK: - looks_like_github_slug

    #[test]
    fn accepts_an_owner_repo_slug() {
        assert!(looks_like_github_slug("Love-Rox/labolabo"));
    }

    #[test]
    fn rejects_a_bare_folder_name() {
        assert!(!looks_like_github_slug("labolabo"));
    }

    #[test]
    fn rejects_an_empty_owner_or_repo() {
        assert!(!looks_like_github_slug("/labolabo"));
        assert!(!looks_like_github_slug("Love-Rox/"));
        assert!(!looks_like_github_slug(""));
    }

    #[test]
    fn rejects_more_than_one_slash() {
        assert!(!looks_like_github_slug("a/b/c"));
    }

    // MARK: - fetch_pr_info (trait-injected fake, no real `gh`/network)

    struct FakeGhPrLister {
        response: RefCell<Option<String>>,
        calls: RefCell<Vec<(String, String)>>,
    }

    impl FakeGhPrLister {
        fn new(response: Option<&str>) -> Self {
            Self {
                response: RefCell::new(response.map(str::to_string)),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl GhPrLister for FakeGhPrLister {
        fn list_prs(&self, branch: &str, repo: &str) -> Option<String> {
            self.calls
                .borrow_mut()
                .push((branch.to_string(), repo.to_string()));
            self.response.borrow().clone()
        }
    }

    #[test]
    fn fetch_pr_info_parses_the_fake_listers_output() {
        let body = r#"[{"number":42,"state":"OPEN","isDraft":false,"title":"t","url":"u"}]"#;
        let lister = FakeGhPrLister::new(Some(body));
        let info = fetch_pr_info(&lister, "feature/x", "o/r").expect("pr");
        assert_eq!(info.number, 42);
        assert_eq!(
            lister.calls.borrow().as_slice(),
            [("feature/x".to_string(), "o/r".to_string())]
        );
    }

    #[test]
    fn fetch_pr_info_none_when_the_lister_fails() {
        let lister = FakeGhPrLister::new(None);
        assert_eq!(fetch_pr_info(&lister, "b", "o/r"), None);
    }

    #[test]
    fn fetch_pr_info_none_when_the_branch_has_no_pr() {
        let lister = FakeGhPrLister::new(Some("[]"));
        assert_eq!(fetch_pr_info(&lister, "b", "o/r"), None);
    }
}
