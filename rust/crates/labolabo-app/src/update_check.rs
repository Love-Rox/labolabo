//! Lightweight update check (RC release wave, `.github/workflows/
//! rust-release.yml`'s counterpart in-app): once per launch, in the
//! background, ask GitHub for the newest LaboLabo-rs release and show a
//! dismissible banner (`crate::sidebar::render_update_banner`) if it's
//! newer than the running binary. A minimal Rust port of the Swift app's
//! `app/Sources/UpdateChecker.swift`, deliberately smaller in scope per the
//! task brief: no periodic re-polling while running (Swift throttles to
//! once per 6h; this port only ever checks once, at startup), and no OS
//! notification (Swift's `AgentNotifier.postUpdateAvailable`) -- just the
//! in-app banner.
//!
//! ## Two release channels, one repo
//!
//! This repo's GitHub Releases mix two independent tag families: the Swift
//! app's release-please-managed `v*` tags (`releases/latest` today always
//! resolves to one of those) and this Rust port's own `rs-v*` tags
//! (`rust-release.yml`, `workflow_dispatch`-only, pre-releases created as
//! **drafts** -- see that workflow's module doc comment for why a draft
//! release's tag isn't actually pushed to the repo until a human publishes
//! it on GitHub, and why an anonymous `curl` here therefore can never see
//! an unpublished RC). [`TAG_PREFIX`] is how every function in this module
//! tells the two apart: the Swift channel is simply ignored wherever it's
//! encountered rather than treated as an error (e.g. [`parse_latest_release`]
//! returns `None` for a `v0.7.x`-tagged `/releases/latest` response, not a
//! parse failure).
//!
//! [`APP_VERSION`](crate::menus::APP_VERSION) currently always contains
//! `-rc` (the RC wave hasn't shipped a final `1.0.0` yet), so
//! [`check_for_update`] always takes the "RC channel" path today
//! ([`is_rc_build`] true -> `/releases?per_page=10`, filtered to the first
//! `rs-v*` entry -- GitHub already returns newest-first). Once a non-RC
//! `1.0.0` ships, the same function switches to the "stable channel" path
//! (`/releases/latest`) automatically -- no code changes needed, this is
//! purely a function of the running binary's own version string.
//!
//! ## Failure handling
//!
//! Every network/parse failure collapses to `None` (curl not on `PATH`,
//! network error, non-2xx status -- curl's own `-f` flag turns those into a
//! nonzero exit -- malformed JSON, missing fields): per the task brief,
//! this check is not allowed to surface any UI (no error banner, no retry
//! affordance), so there is nothing meaningful to distinguish a caller from
//! "no update available" in the first place.
//!
//! ## Version comparison and the RC gap in `release_version`
//!
//! [`labolabo_core::release_version`] is a faithful port of the Swift app's
//! purely-dotted-numeric comparator; it does not understand `-rc.N`
//! suffixes as pre-release markers (a segment like `0-rc` is parsed by
//! taking only its leading digit run, so `"1.0.0-rc.1"` and `"1.0.0"`
//! compare as `[1,0,0,1]` vs `[1,0,0,0]` -- the *rc build* reads as
//! "newer", **in either direction you ask it**: `is_newer("1.0.0",
//! "1.0.0-rc.1")` is false, but so is `is_newer("1.0.0-rc.1", "1.0.0")`
//! *should* be -- wrongly returns true, since `[1,0,0,1] > [1,0,0,0]`
//! either way you frame the comparison). This is exactly right for
//! comparing two RC numbers of the *same* base against each other
//! (`"1.0.0-rc.2"` vs `"1.0.0-rc.1"` -> `[1,0,0,2]` vs `[1,0,0,1]`,
//! correctly newer), but wrong for "does a pre-release suffix outrank no
//! suffix at all" whenever the X.Y.Z base is identical.
//! [`is_update_available`] special-cases exactly that one comparison
//! (same base, exactly one side has a `-rc...` suffix) instead of
//! deferring to `release_version::is_newer` -- see its doc comment.

use std::path::Path;
use std::time::Duration;

use labolabo_core::release_version;

/// `owner/repo` slug this port's release pipeline lives in -- also the
/// Swift app's own repo (`app/Sources/GitHubRepo.swift`'s `slug`), which is
/// exactly why [`TAG_PREFIX`] matters (see this module's doc comment).
pub const REPO_SLUG: &str = "Love-Rox/labolabo";

const RELEASES_LATEST_URL: &str = "https://api.github.com/repos/Love-Rox/labolabo/releases/latest";
/// `per_page=10` comfortably covers "most recent RC" even if a few Swift
/// `v*` releases were interleaved chronologically -- GitHub returns newest
/// first regardless of tag family.
const RELEASES_LIST_URL: &str =
    "https://api.github.com/repos/Love-Rox/labolabo/releases?per_page=10";
/// This Rust port's own tag prefix (`rust-release.yml`'s `tag` input,
/// e.g. `rs-v1.0.0-rc.1`) -- distinct from the Swift app's release-please
/// `v*` tags on the same repo (see this module's doc comment).
const TAG_PREFIX: &str = "rs-v";
/// Matches the Swift `UpdateChecker`'s own `URLRequest.timeoutInterval`
/// (15s) closely enough while staying well under a launch-blocking delay --
/// this check always runs in the background (`app.rs`'s startup
/// `cx.spawn`/`cx.background_spawn`), so it never blocks the window from
/// opening, but an unbounded `curl` could still leave a background thread
/// hung indefinitely on a stalled connection.
const CURL_TIMEOUT: Duration = Duration::from_secs(5);

/// A GitHub release, reduced to just what the banner needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    /// Normalized (no `rs-v`/`v` prefix -- [`release_version::normalize`]),
    /// e.g. `"1.0.0-rc.2"`.
    pub version: String,
    /// The release's GitHub page, or the repo's releases list if the API
    /// response omitted `html_url` (never lets a missing URL make the
    /// banner's "開く" button silently do nothing).
    pub url: String,
}

/// `true` once [`crate::menus::APP_VERSION`] contains an `-rc` pre-release
/// marker -- see this module's doc comment for which release-check
/// endpoint/filter this selects.
pub fn is_rc_build(version: &str) -> bool {
    version.contains("-rc")
}

fn strip_tag_prefix(tag: &str) -> &str {
    tag.strip_prefix(TAG_PREFIX).unwrap_or(tag)
}

/// Splits `"1.2.3-rc.4"` into base `"1.2.3"` and suffix `Some("rc.4")`;
/// `"1.2.3"` (no `-`) -> `("1.2.3", None)`.
fn split_prerelease(version: &str) -> (&str, Option<&str>) {
    match version.split_once('-') {
        Some((base, suffix)) => (base, Some(suffix)),
        None => (version, None),
    }
}

/// `true` if `remote_version` (a normalized tag, e.g. from [`ReleaseInfo`])
/// should be considered an update over `current_version`
/// ([`crate::menus::APP_VERSION`]).
///
/// When `remote`/`current` share the same X.Y.Z base and exactly one side
/// has a `-rc...` pre-release suffix, that side's suffix-vs-no-suffix
/// status alone decides the answer (a suffix always ranks *below* no
/// suffix at the same base) -- overriding what `release_version::is_newer`
/// would say either direction (see this module's doc comment for why it
/// gets both directions of this specific comparison backwards). Every
/// other case (different bases, or both/neither side suffixed) defers
/// straight to [`release_version::is_newer`], which is already correct
/// there (including comparing two RC numbers of the same base against each
/// other).
pub fn is_update_available(remote_version: &str, current_version: &str) -> bool {
    let remote = release_version::normalize(remote_version);
    let current = release_version::normalize(current_version);
    if remote == current {
        return false;
    }
    let (remote_base, remote_suffix) = split_prerelease(&remote);
    let (current_base, current_suffix) = split_prerelease(&current);
    if release_version::compare(remote_base, current_base) == 0 {
        match (remote_suffix.is_some(), current_suffix.is_some()) {
            // Final release supersedes any RC of the same base.
            (false, true) => return true,
            // An RC of your current stable's base is never ahead of it.
            (true, false) => return false,
            // Both (or neither, though `remote == current` already caught
            // that) suffixed -- release_version::is_newer compares the
            // `-rc.N` segment correctly here, see below.
            _ => {}
        }
    }
    release_version::is_newer(&remote, &current)
}

fn release_from_value(value: &serde_json::Value) -> Option<ReleaseInfo> {
    let tag = value.get("tag_name")?.as_str()?;
    if !tag.starts_with(TAG_PREFIX) {
        // A Swift-channel `v*` tag (or something unrecognized) -- not an
        // error, just not this port's release (see module doc comment).
        return None;
    }
    let version = release_version::normalize(strip_tag_prefix(tag));
    let url = value
        .get("html_url")
        .and_then(|u| u.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://github.com/{REPO_SLUG}/releases"));
    Some(ReleaseInfo { version, url })
}

/// Parses a GitHub `GET /repos/{owner}/{repo}/releases/latest` response
/// body (a single JSON object). `None` on malformed JSON, missing
/// `tag_name`, or a non-`rs-v*` tag (the Swift channel -- see module doc
/// comment for why that's "ignore", not "error").
pub fn parse_latest_release(body: &str) -> Option<ReleaseInfo> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    release_from_value(&value)
}

/// Parses a GitHub `GET /repos/{owner}/{repo}/releases?per_page=N` response
/// body (a JSON array, newest first) and returns the first `rs-v*`-tagged
/// entry. `None` if the JSON is malformed, isn't an array, or contains no
/// `rs-v*` entry within the fetched page.
pub fn parse_first_rs_release(body: &str) -> Option<ReleaseInfo> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let array = value.as_array()?;
    array.iter().find_map(release_from_value)
}

/// Blocking: runs `curl -fsSL --max-time 5 -H "Accept: ..." <url>` and
/// returns stdout on success. `None` on *any* failure -- `curl` missing
/// from `PATH`, a network error, or (`-f`) a non-2xx HTTP status -- per
/// this module's "collapses to silence" contract. Must be called from a
/// background thread (`labolabo_core::process::run_with_timeout` blocks the
/// calling thread -- see that function's module doc comment).
fn curl_get(url: &str) -> Option<String> {
    let args = [
        "-fsSL".to_string(),
        "--max-time".to_string(),
        CURL_TIMEOUT.as_secs().to_string(),
        "-H".to_string(),
        "Accept: application/vnd.github+json".to_string(),
        url.to_string(),
    ];
    // A little slack past curl's own `--max-time` so curl (not this outer
    // guard) is normally the one to end a stalled connection -- the outer
    // timeout only matters if curl itself somehow ignores `--max-time`
    // (e.g. hung DNS resolution before the transfer timer starts).
    let outer_timeout = CURL_TIMEOUT + Duration::from_secs(3);
    match labolabo_core::process::run_with_timeout(
        Path::new("curl"),
        &args,
        None,
        None,
        outer_timeout,
    ) {
        Ok(Some(output)) if output.status == 0 => Some(output.stdout),
        _ => None,
    }
}

/// Runs the actual network check (blocking -- call from a background
/// thread, e.g. `cx.background_spawn`). Returns `Some` only when a genuine
/// update is available for `current_version` (already filtered through
/// [`is_update_available`]); every failure or "already up to date" case
/// collapses to `None`, per this module's silent-failure contract.
pub fn check_for_update(current_version: &str) -> Option<ReleaseInfo> {
    let release = if is_rc_build(current_version) {
        let body = curl_get(RELEASES_LIST_URL)?;
        parse_first_rs_release(&body)?
    } else {
        let body = curl_get(RELEASES_LATEST_URL)?;
        parse_latest_release(&body)?
    };
    if is_update_available(&release.version, current_version) {
        Some(release)
    } else {
        None
    }
}

/// Decides whether a freshly-fetched `release` should actually surface a
/// banner, given the persisted "don't notify this version again" state
/// (`TaskDatabase::ignored_update_version`, written by
/// `LaboLaboApp::dismiss_update_banner` -- see that method's doc comment
/// for why dismissing the banner *is* this task brief's "今後このバージョン
/// を通知しない"). `None`/a different version than what was last dismissed
/// both mean "yes, show it" -- only an exact match is suppressed, so a
/// newer release than the one the user dismissed still notifies.
pub fn should_notify(release: &ReleaseInfo, ignored_version: Option<&str>) -> bool {
    ignored_version != Some(release.version.as_str())
}

/// Env-var kill switch for the whole check (task brief: `LABOLABO_NO_
/// UPDATE_CHECK=1`), independent of the settings-screen toggle -- primarily
/// for the smoke-test/CI/dev-run path (`rust/README.md`'s smoke-run
/// instructions), which must never make a real network call. Same
/// "env-reading wrapper around a pure `_from` helper" shape as
/// `crate::motion::reduce_motion`.
pub fn update_check_disabled() -> bool {
    update_check_disabled_from(std::env::var("LABOLABO_NO_UPDATE_CHECK").ok())
}

fn update_check_disabled_from(value: Option<String>) -> bool {
    value.as_deref() == Some("1")
}

/// Opens `url` in the user's default browser -- the update banner's
/// "開く"/"Open" button. Blocking (spawn a real OS process); callers must
/// run this from a background thread, same contract as `crate::ide_open`'s
/// `open_in_editor`/`reveal_in_finder`.
///
/// Cross-platform (unlike `crate::ide_open`, which is macOS-only): `open`
/// on macOS (`NSWorkspace.open` via LaunchServices, same as Swift's own
/// `UpdateChecker`), `xdg-open` on Linux (the freedesktop.org convention
/// every major desktop environment provides), `cmd /C start "" <url>` on
/// Windows (`start`'s own leading `""` is its window-title placeholder --
/// required so `start` doesn't mistake a quoted URL for that title; see
/// e.g. Microsoft's `start /?` docs). The Windows path is unverified on a
/// real desktop (this repo has no Windows machine in its dev loop -- same
/// caveat `rust/scripts/package-windows.ps1`'s README carries for the rest
/// of the Windows build).
pub fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let (program, args): (&str, Vec<String>) = ("/usr/bin/open", vec![url.to_string()]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let (program, args): (&str, Vec<String>) = ("xdg-open", vec![url.to_string()]);
    #[cfg(target_os = "windows")]
    let (program, args): (&str, Vec<String>) = (
        "cmd",
        vec![
            "/C".to_string(),
            "start".to_string(),
            String::new(),
            url.to_string(),
        ],
    );

    match labolabo_core::process::run(Path::new(program), &args, None, None) {
        Ok(output) if output.status == 0 => Ok(()),
        Ok(output) => Err(format!(
            "{program} failed (exit {}): {}",
            output.status,
            output.stderr.trim()
        )),
        Err(err) => Err(format!("failed to launch {program}: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - is_rc_build

    #[test]
    fn is_rc_build_detects_the_rc_marker() {
        assert!(is_rc_build("1.0.0-rc.1"));
        assert!(!is_rc_build("1.0.0"));
        assert!(!is_rc_build("0.9.5"));
    }

    // MARK: - is_update_available

    #[test]
    fn newer_rc_number_of_the_same_base_is_an_update() {
        assert!(is_update_available("1.0.0-rc.2", "1.0.0-rc.1"));
        assert!(!is_update_available("1.0.0-rc.1", "1.0.0-rc.1"));
        assert!(!is_update_available("1.0.0-rc.1", "1.0.0-rc.2"));
    }

    #[test]
    fn higher_base_version_is_an_update_regardless_of_suffix() {
        assert!(is_update_available("1.1.0", "1.0.0-rc.1"));
        assert!(is_update_available("2.0.0-rc.1", "1.0.0-rc.5"));
        assert!(!is_update_available("0.9.0", "1.0.0-rc.1"));
    }

    #[test]
    fn final_release_supersedes_any_rc_of_the_same_base() {
        // The exact gap `release_version`'s pure numeric compare misses
        // (module doc comment): release_version::is_newer("1.0.0",
        // "1.0.0-rc.1") is false, but a final 1.0.0 *is* an update over a
        // running 1.0.0-rc.1.
        assert!(!release_version::is_newer("1.0.0", "1.0.0-rc.1"));
        assert!(is_update_available("1.0.0", "1.0.0-rc.1"));
        assert!(is_update_available("1.0.0", "1.0.0-rc.9"));
    }

    #[test]
    fn identical_versions_are_never_an_update() {
        assert!(!is_update_available("1.0.0-rc.1", "1.0.0-rc.1"));
        assert!(!is_update_available("v1.0.0", "1.0.0")); // normalize() strips "v"
    }

    #[test]
    fn a_stable_current_build_is_not_updated_by_an_older_rc() {
        // The reverse direction of `final_release_supersedes_any_rc_of_
        // the_same_base` above: current is already the final 1.0.0; a
        // stray 1.0.0-rc.1 "release" of the same base (shouldn't normally
        // happen -- GitHub's `/releases/latest` excludes prereleases in
        // the first place -- but this function must be correct on its own
        // regardless of caller discipline) is not an update. Without the
        // explicit suffix check in `is_update_available`,
        // `release_version::is_newer("1.0.0-rc.1", "1.0.0")` alone would
        // wrongly say `true` here (this module's doc comment).
        assert!(release_version::is_newer("1.0.0-rc.1", "1.0.0")); // the quirk, in isolation
        assert!(!is_update_available("1.0.0-rc.1", "1.0.0"));
    }

    // MARK: - parse_latest_release / parse_first_rs_release

    #[test]
    fn parses_a_latest_release_response_with_our_tag_prefix() {
        let body = r#"{"tag_name":"rs-v1.0.0-rc.2","html_url":"https://github.com/Love-Rox/labolabo/releases/tag/rs-v1.0.0-rc.2"}"#;
        let release = parse_latest_release(body).expect("release");
        assert_eq!(release.version, "1.0.0-rc.2");
        assert_eq!(
            release.url,
            "https://github.com/Love-Rox/labolabo/releases/tag/rs-v1.0.0-rc.2"
        );
    }

    #[test]
    fn ignores_a_swift_channel_tag_on_the_latest_endpoint() {
        // `/releases/latest` resolving to the Swift app's own `v*` tag --
        // must not be mistaken for a Rust-port release.
        let body = r#"{"tag_name":"v0.7.0","html_url":"https://github.com/Love-Rox/labolabo/releases/tag/v0.7.0"}"#;
        assert_eq!(parse_latest_release(body), None);
    }

    #[test]
    fn falls_back_to_the_releases_page_when_html_url_is_missing() {
        let body = r#"{"tag_name":"rs-v1.0.0-rc.1"}"#;
        let release = parse_latest_release(body).expect("release");
        assert_eq!(release.url, "https://github.com/Love-Rox/labolabo/releases");
    }

    #[test]
    fn malformed_json_yields_none() {
        assert_eq!(parse_latest_release("not json"), None);
        assert_eq!(parse_latest_release(""), None);
        assert_eq!(parse_latest_release(r#"{"no_tag_name":true}"#), None);
    }

    #[test]
    fn parse_first_rs_release_picks_the_first_matching_entry_in_a_list() {
        let body = r#"[
            {"tag_name":"v0.7.1","html_url":"https://example.com/v0.7.1"},
            {"tag_name":"rs-v1.0.0-rc.3","html_url":"https://example.com/rc3"},
            {"tag_name":"rs-v1.0.0-rc.2","html_url":"https://example.com/rc2"}
        ]"#;
        let release = parse_first_rs_release(body).expect("release");
        assert_eq!(release.version, "1.0.0-rc.3");
        assert_eq!(release.url, "https://example.com/rc3");
    }

    #[test]
    fn parse_first_rs_release_returns_none_when_the_page_has_no_rs_tag() {
        let body = r#"[{"tag_name":"v0.7.1"},{"tag_name":"v0.7.0"}]"#;
        assert_eq!(parse_first_rs_release(body), None);
    }

    #[test]
    fn parse_first_rs_release_rejects_a_non_array_body() {
        assert_eq!(parse_first_rs_release(r#"{"tag_name":"rs-v1.0.0"}"#), None);
    }

    // MARK: - should_notify

    #[test]
    fn should_notify_true_when_never_dismissed() {
        let release = ReleaseInfo {
            version: "1.0.0-rc.2".to_string(),
            url: "https://example.com".to_string(),
        };
        assert!(should_notify(&release, None));
    }

    #[test]
    fn should_notify_false_for_an_exactly_dismissed_version() {
        let release = ReleaseInfo {
            version: "1.0.0-rc.2".to_string(),
            url: "https://example.com".to_string(),
        };
        assert!(!should_notify(&release, Some("1.0.0-rc.2")));
    }

    #[test]
    fn should_notify_true_for_a_newer_version_than_the_dismissed_one() {
        let release = ReleaseInfo {
            version: "1.0.0-rc.3".to_string(),
            url: "https://example.com".to_string(),
        };
        assert!(should_notify(&release, Some("1.0.0-rc.2")));
    }

    // MARK: - update_check_disabled

    #[test]
    fn update_check_disabled_from_env() {
        assert!(!update_check_disabled_from(None));
        assert!(!update_check_disabled_from(Some("0".to_string())));
        assert!(!update_check_disabled_from(Some("".to_string())));
        assert!(update_check_disabled_from(Some("1".to_string())));
    }
}
