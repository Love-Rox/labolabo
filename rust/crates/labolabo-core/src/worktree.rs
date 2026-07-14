//! Faithful port of `Sources/LaboLaboEngine/Git/Worktree.swift`.
//!
//! Parser for `git worktree list --porcelain`.

/// One entry from `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub is_detached: bool,
    pub is_locked: bool,
    pub is_bare: bool,
}

impl Worktree {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            head: None,
            branch: None,
            is_detached: false,
            is_locked: false,
            is_bare: false,
        }
    }

    /// Short branch name (`refs/heads/feature/x` -> `feature/x`).
    pub fn short_branch(&self) -> Option<&str> {
        let branch = self.branch.as_deref()?;
        Some(branch.strip_prefix("refs/heads/").unwrap_or(branch))
    }
}

/// Parses the blank-line-separated blocks of `git worktree list --porcelain`.
pub fn parse(raw: &str) -> Vec<Worktree> {
    let mut worktrees: Vec<Worktree> = Vec::new();
    let mut current: Option<Worktree> = None;

    // `split(separator: "\n", omittingEmptySubsequences: false)`: blank lines
    // are kept as empty elements — they are exactly the block separators.
    for line in raw.split('\n') {
        if line.is_empty() {
            if let Some(c) = current.take() {
                worktrees.push(c);
            }
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        let key = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("");
        match key {
            "worktree" => {
                if let Some(c) = current.take() {
                    worktrees.push(c);
                }
                current = Some(Worktree::new(value));
            }
            "HEAD" => {
                if let Some(c) = current.as_mut() {
                    c.head = Some(value.to_string());
                }
            }
            "branch" => {
                if let Some(c) = current.as_mut() {
                    c.branch = Some(value.to_string());
                }
            }
            "detached" => {
                if let Some(c) = current.as_mut() {
                    c.is_detached = true;
                }
            }
            "locked" => {
                if let Some(c) = current.as_mut() {
                    c.is_locked = true;
                }
            }
            "bare" => {
                if let Some(c) = current.as_mut() {
                    c.is_bare = true;
                }
            }
            _ => {}
        }
    }
    if let Some(c) = current.take() {
        worktrees.push(c);
    }
    worktrees
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported 1:1 from Tests/LaboLaboEngineTests/WorktreeListParserTests.swift.

    #[test]
    fn parses_blocks() {
        let raw = "worktree /repo\n\
HEAD aaaaaaa\n\
branch refs/heads/main\n\
\n\
worktree /repo/.worktrees/x\n\
HEAD bbbbbbb\n\
branch refs/heads/feature/x\n\
\n\
worktree /repo/locked-detached\n\
HEAD ccccccc\n\
detached\n\
locked";
        let worktrees = parse(raw);
        assert_eq!(worktrees.len(), 3);

        assert_eq!(worktrees[0].path, "/repo");
        assert_eq!(worktrees[0].short_branch(), Some("main"));
        assert!(!worktrees[0].is_detached);

        assert_eq!(worktrees[1].short_branch(), Some("feature/x"));

        assert!(worktrees[2].is_detached);
        assert!(worktrees[2].is_locked);
        assert_eq!(worktrees[2].short_branch(), None);
    }
}
