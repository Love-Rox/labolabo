//! Faithful port of `Sources/LaboLaboEngine/Git/PorcelainStatusParser.swift`.
//!
//! Parser for `git status --porcelain=v2 --branch -z`.
//!
//! Records are NUL-separated. Rename/copy (type `2`) entries store the
//! original path in the *following* NUL token, so the tokenizer must
//! consume two tokens for those.
//! See: <https://git-scm.com/docs/git-status#_porcelain_format_version_2>

use crate::git_models::{Change, GitFileEntry, GitStatus, Kind};
use crate::util::{drop_first_chars, split_space_omitting_empty};

pub fn parse(raw: &str) -> GitStatus {
    let mut status = GitStatus::default();
    // NUL-separated, omitting empty subsequences (mirrors Swift's
    // `split(separator: "\u{0}", omittingEmptySubsequences: true)`).
    let tokens: Vec<&str> = raw.split('\0').filter(|t| !t.is_empty()).collect();

    let mut i = 0usize;
    while i < tokens.len() {
        let token = tokens[i];
        match token.chars().next() {
            Some('#') => {
                parse_header(token, &mut status);
                i += 1;
            }
            Some('1') => {
                if let Some(entry) = parse_ordinary(token) {
                    status.entries.push(entry);
                }
                i += 1;
            }
            Some('2') => {
                // The original path lives in the *next* token regardless of
                // whether this record itself parses successfully.
                let original = tokens.get(i + 1).copied();
                if let Some(entry) = parse_rename_copy(token, original) {
                    status.entries.push(entry);
                }
                i += 2;
            }
            Some('u') => {
                if let Some(entry) = parse_unmerged(token) {
                    status.entries.push(entry);
                }
                i += 1;
            }
            Some('?') => {
                status.entries.push(GitFileEntry::new(
                    Kind::Untracked,
                    drop_first_chars(token, 2),
                ));
                i += 1;
            }
            Some('!') => {
                status
                    .entries
                    .push(GitFileEntry::new(Kind::Ignored, drop_first_chars(token, 2)));
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    status
}

// MARK: - Header

fn parse_header(token: &str, status: &mut GitStatus) {
    let parts = split_space_omitting_empty(token);
    if parts.len() < 3 {
        return;
    }
    match parts[1] {
        "branch.oid" => {
            status.head_sha = if parts[2] == "(initial)" {
                None
            } else {
                Some(parts[2].to_string())
            };
        }
        "branch.head" => {
            status.branch = Some(parts[2].to_string());
        }
        "branch.upstream" => {
            status.upstream = Some(parts[2].to_string());
        }
        "branch.ab" if parts.len() >= 4 => {
            // "+N" / "-M"
            status.ahead = drop_first_chars(parts[2], 1).parse().unwrap_or(0);
            status.behind = drop_first_chars(parts[3], 1).parse().unwrap_or(0);
        }
        _ => {}
    }
}

// MARK: - Entries

/// `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`
fn parse_ordinary(token: &str) -> Option<GitFileEntry> {
    let f: Vec<&str> = token.splitn(9, ' ').collect();
    if f.len() < 9 {
        return None;
    }
    let (index, worktree) = xy_pair(f[1])?;
    Some(GitFileEntry {
        kind: Kind::Ordinary,
        index,
        worktree,
        path: f[8].to_string(),
        original_path: None,
        score: None,
    })
}

/// `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <Xscore> <path>` (+ original path in next token)
fn parse_rename_copy(token: &str, original_path: Option<&str>) -> Option<GitFileEntry> {
    let f: Vec<&str> = token.splitn(10, ' ').collect();
    if f.len() < 10 {
        return None;
    }
    let (index, worktree) = xy_pair(f[1])?;
    let xscore = f[8]; // e.g. "R100" / "C75"
    let score = if xscore.chars().count() >= 2 {
        drop_first_chars(xscore, 1).parse::<i64>().ok()
    } else {
        None
    };
    Some(GitFileEntry {
        kind: Kind::RenamedOrCopied,
        index,
        worktree,
        path: f[9].to_string(),
        original_path: original_path.map(|s| s.to_string()),
        score,
    })
}

/// `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`
fn parse_unmerged(token: &str) -> Option<GitFileEntry> {
    let f: Vec<&str> = token.splitn(11, ' ').collect();
    if f.len() < 11 {
        return None;
    }
    let (index, worktree) = xy_pair(f[1])?;
    Some(GitFileEntry {
        kind: Kind::Unmerged,
        index,
        worktree,
        path: f[10].to_string(),
        original_path: None,
        score: None,
    })
}

fn xy_pair(field: &str) -> Option<(Change, Change)> {
    let chars: Vec<char> = field.chars().collect();
    if chars.len() != 2 {
        return None;
    }
    Some((
        Change::from_porcelain(chars[0]),
        Change::from_porcelain(chars[1]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors `PorcelainStatusParserTests.nulJoined`.
    fn nul_joined(lines: &[&str]) -> String {
        let mut s = lines.join("\0");
        s.push('\0');
        s
    }

    // Ported 1:1 from Tests/LaboLaboEngineTests/PorcelainStatusParserTests.swift.

    #[test]
    fn branch_headers_and_ahead_behind() {
        let raw = nul_joined(&[
            "# branch.oid abc123",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
        ]);
        let status = parse(&raw);
        assert_eq!(status.head_sha.as_deref(), Some("abc123"));
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!(status.ahead, 2);
        assert_eq!(status.behind, 1);
        assert!(!status.is_detached());
    }

    #[test]
    fn detached_head() {
        let raw = nul_joined(&["# branch.head (detached)"]);
        assert!(parse(&raw).is_detached());
    }

    #[test]
    fn ordinary_staged_unstaged_and_path_with_space() {
        let raw = nul_joined(&[
            "# branch.head main",
            "1 .M N... 100644 100644 100644 1111111 2222222 src/foo.swift",
            "1 M. N... 100644 100644 100644 3333333 4444444 src/bar baz.swift",
        ]);
        let status = parse(&raw);
        assert_eq!(status.entries.len(), 2);

        let foo = &status.entries[0];
        assert_eq!(foo.path, "src/foo.swift");
        assert_eq!(foo.index, Change::Unmodified);
        assert_eq!(foo.worktree, Change::Modified);
        assert!(foo.is_unstaged());
        assert!(!foo.is_staged());

        let bar = &status.entries[1];
        assert_eq!(
            bar.path, "src/bar baz.swift",
            "spaces in paths must be preserved"
        );
        assert_eq!(bar.index, Change::Modified);
        assert!(bar.is_staged());

        assert_eq!(
            status
                .staged()
                .iter()
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>(),
            vec!["src/bar baz.swift"]
        );
        assert_eq!(
            status
                .unstaged()
                .iter()
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>(),
            vec!["src/foo.swift"]
        );
    }

    #[test]
    fn rename_consumes_original_path_token() {
        let raw = nul_joined(&[
            "# branch.head main",
            "2 R. N... 100644 100644 100644 5555555 6666666 R100 new/name.swift",
            "old/name.swift",
            "? untracked.txt",
        ]);
        let status = parse(&raw);
        assert_eq!(
            status.entries.len(),
            2,
            "rename + untracked; the original-path token must not become its own entry"
        );

        let rename = &status.entries[0];
        assert_eq!(rename.kind, Kind::RenamedOrCopied);
        assert_eq!(rename.path, "new/name.swift");
        assert_eq!(rename.original_path.as_deref(), Some("old/name.swift"));
        assert_eq!(rename.score, Some(100));
        assert_eq!(rename.index, Change::Renamed);

        assert_eq!(
            status
                .untracked()
                .iter()
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>(),
            vec!["untracked.txt"]
        );
    }

    #[test]
    fn clean_repo_is_not_dirty() {
        let raw = nul_joined(&["# branch.head main", "# branch.ab +0 -0"]);
        assert!(!parse(&raw).is_dirty());
    }

    // Additional edge cases not covered by the existing Swift test suite but
    // exercised by the shared golden fixtures too (see fixtures/inputs/porcelain).

    #[test]
    fn empty_input_yields_default_status() {
        let status = parse("");
        assert_eq!(status, GitStatus::default());
    }

    #[test]
    fn initial_commit_has_no_head_sha() {
        let raw = nul_joined(&["# branch.oid (initial)", "# branch.head main"]);
        assert_eq!(parse(&raw).head_sha, None);
    }

    #[test]
    fn unmerged_conflict_is_conflicted_and_unstaged_not_staged() {
        let raw = nul_joined(&[
            "# branch.head main",
            "u UU N... 100644 100644 100644 100644 1111111 2222222 3333333 conflict.txt",
        ]);
        let status = parse(&raw);
        assert_eq!(status.entries.len(), 1);
        let entry = &status.entries[0];
        assert_eq!(entry.kind, Kind::Unmerged);
        assert_eq!(entry.path, "conflict.txt");
        assert!(entry.is_unstaged());
        assert!(!entry.is_staged());
        assert_eq!(
            status
                .conflicted()
                .iter()
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>(),
            vec!["conflict.txt"]
        );
    }

    #[test]
    fn ignored_entries_are_not_dirty_contributors_when_alone() {
        let raw = nul_joined(&["# branch.head main", "! build/output.log"]);
        let status = parse(&raw);
        assert_eq!(status.entries.len(), 1);
        assert_eq!(status.entries[0].kind, Kind::Ignored);
        // isDirty is true whenever *any* entry has kind != .ignored; an
        // ignored-only status is therefore not dirty.
        assert!(!status.is_dirty());
    }

    #[test]
    fn untracked_entry_alone_counts_as_dirty() {
        // Documents a real (if easy to overlook) Swift behavior: isDirty
        // does not mean "has staged or unstaged changes" -- a purely
        // untracked file also makes the repo "dirty".
        let raw = nul_joined(&["# branch.head main", "? scratch.txt"]);
        assert!(parse(&raw).is_dirty());
    }

    #[test]
    fn malformed_ordinary_record_is_dropped_not_fatal() {
        let raw = nul_joined(&[
            "# branch.head main",
            "1 .M N... 100644 100644", // too few fields
            "? still_here.txt",
        ]);
        let status = parse(&raw);
        assert_eq!(status.entries.len(), 1);
        assert_eq!(status.entries[0].path, "still_here.txt");
    }

    #[test]
    fn unknown_marker_is_skipped() {
        let raw = nul_joined(&[
            "# branch.head main",
            "x this is not a recognised record",
            "? after.txt",
        ]);
        let status = parse(&raw);
        assert_eq!(status.entries.len(), 1);
        assert_eq!(status.entries[0].path, "after.txt");
    }

    #[test]
    fn invalid_porcelain_change_char_falls_back_to_unmodified() {
        assert_eq!(Change::from_porcelain('Z'), Change::Unmodified);
    }
}
