//! Faithful port of `Sources/LaboLaboEngine/Git/CrossSessionConflicts.swift`.
//!
//! Detects "editing the same file concurrently" across sessions (pure
//! function). 1 session = 1 worktree, but worktrees sharing the same repo
//! (shared `.git`) can touch the same relative path on different branches.
//! This enumerates, among sessions with the same `repo_key`, changed paths
//! that overlap — for the UI's conflict warning.

use std::collections::HashSet;

/// One session's input (id, owning repo, set of changed paths).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    /// Stable key for the owning repo (shared git directory). `None` if unresolved.
    pub repo_key: Option<String>,
    /// Changed paths, relative to the worktree root.
    pub changed: HashSet<String>,
}

impl Session {
    pub fn new(id: impl Into<String>, repo_key: Option<String>, changed: HashSet<String>) -> Self {
        Self {
            id: id.into(),
            repo_key,
            changed,
        }
    }
}

/// A single file's conflict (path, and the other session ids sharing it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: String,
    pub others: Vec<String>,
}

/// Paths that the `id` session has changed *and* that at least one other
/// session with the **same `repo_key`** has also changed. Sorted by path
/// ascending; `others` is in the input order of the other sessions.
pub fn conflicts(id: &str, sessions: &[Session]) -> Vec<Conflict> {
    let Some(me) = sessions.iter().find(|s| s.id == id) else {
        return Vec::new();
    };
    let Some(repo_key) = me.repo_key.as_deref() else {
        return Vec::new();
    };
    if me.changed.is_empty() {
        return Vec::new();
    }

    let siblings: Vec<&Session> = sessions
        .iter()
        .filter(|s| s.id != id && s.repo_key.as_deref() == Some(repo_key))
        .collect();
    if siblings.is_empty() {
        return Vec::new();
    }

    let mut paths: Vec<&String> = me.changed.iter().collect();
    paths.sort();

    paths
        .into_iter()
        .filter_map(|path| {
            let others: Vec<String> = siblings
                .iter()
                .filter(|s| s.changed.contains(path))
                .map(|s| s.id.clone())
                .collect();
            if others.is_empty() {
                None
            } else {
                Some(Conflict {
                    path: path.clone(),
                    others,
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // Ported 1:1 from Tests/LaboLaboEngineTests/CrossSessionConflictsTests.swift.

    #[test]
    fn detects_shared_path_in_same_repo() {
        let sessions = vec![
            Session::new("a", Some("R".to_string()), set(&["src/foo.swift", "a.txt"])),
            Session::new("b", Some("R".to_string()), set(&["src/foo.swift", "b.txt"])),
        ];
        let result = conflicts("a", &sessions);
        assert_eq!(
            result,
            vec![Conflict {
                path: "src/foo.swift".to_string(),
                others: vec!["b".to_string()],
            }]
        );
    }

    #[test]
    fn no_conflict_across_different_repos() {
        // 同じパスでも repoKey が違えば衝突ではない。
        let sessions = vec![
            Session::new("a", Some("R1".to_string()), set(&["foo.swift"])),
            Session::new("b", Some("R2".to_string()), set(&["foo.swift"])),
        ];
        assert!(conflicts("a", &sessions).is_empty());
    }

    #[test]
    fn no_conflict_when_alone() {
        let sessions = vec![Session::new(
            "a",
            Some("R".to_string()),
            set(&["foo.swift"]),
        )];
        assert!(conflicts("a", &sessions).is_empty());
    }

    #[test]
    fn unresolved_repo_key_is_ignored() {
        // repoKey 未解決（nil）は同居判定に含めない。
        let sessions = vec![
            Session::new("a", None, set(&["foo.swift"])),
            Session::new("b", None, set(&["foo.swift"])),
        ];
        assert!(conflicts("a", &sessions).is_empty());
    }

    #[test]
    fn multiple_others_and_sorted_paths() {
        let sessions = vec![
            Session::new("a", Some("R".to_string()), set(&["z.swift", "a.swift"])),
            Session::new("b", Some("R".to_string()), set(&["a.swift"])),
            Session::new("c", Some("R".to_string()), set(&["a.swift", "z.swift"])),
        ];
        let result = conflicts("a", &sessions);
        // パスは昇順、others は入力順（b, c）。
        assert_eq!(
            result,
            vec![
                Conflict {
                    path: "a.swift".to_string(),
                    others: vec!["b".to_string(), "c".to_string()],
                },
                Conflict {
                    path: "z.swift".to_string(),
                    others: vec!["c".to_string()],
                },
            ]
        );
    }

    #[test]
    fn empty_changed_set_has_no_conflict() {
        let sessions = vec![
            Session::new("a", Some("R".to_string()), set(&[])),
            Session::new("b", Some("R".to_string()), set(&["foo.swift"])),
        ];
        assert!(conflicts("a", &sessions).is_empty());
    }
}
