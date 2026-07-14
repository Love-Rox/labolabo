//! Faithful port of `Sources/LaboLaboEngine/Git/GitModels.swift`.
//!
//! These are plain data types plus the small set of derived (computed)
//! properties the Swift version exposes. Behavior — including which entries
//! count as "staged"/"unstaged"/"dirty" — must match the Swift source
//! exactly; see the doc comments below for the specific rules carried over.

/// Parsed snapshot of `git status --porcelain=v2 --branch -z` for one worktree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitStatus {
    pub head_sha: Option<String>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub entries: Vec<GitFileEntry>,
}

impl GitStatus {
    /// `true` when the branch is in a detached-HEAD state (porcelain reports `(detached)`).
    pub fn is_detached(&self) -> bool {
        self.branch.as_deref() == Some("(detached)")
    }

    /// Files with index-side (staged) changes.
    pub fn staged(&self) -> Vec<&GitFileEntry> {
        self.entries.iter().filter(|e| e.is_staged()).collect()
    }

    /// Files with worktree-side (unstaged) changes.
    pub fn unstaged(&self) -> Vec<&GitFileEntry> {
        self.entries.iter().filter(|e| e.is_unstaged()).collect()
    }

    /// Untracked paths.
    pub fn untracked(&self) -> Vec<&GitFileEntry> {
        self.entries
            .iter()
            .filter(|e| e.kind == Kind::Untracked)
            .collect()
    }

    /// Conflicted (unmerged) paths.
    pub fn conflicted(&self) -> Vec<&GitFileEntry> {
        self.entries
            .iter()
            .filter(|e| e.kind == Kind::Unmerged)
            .collect()
    }

    /// NOTE: this intentionally counts *untracked* entries as "dirty" too —
    /// it is `true` whenever any entry's kind is not `.ignored`, not just
    /// staged/unstaged ones. This mirrors the Swift `isDirty` exactly.
    pub fn is_dirty(&self) -> bool {
        self.entries.iter().any(|e| e.kind != Kind::Ignored)
    }
}

/// A single changed-path record from porcelain v2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFileEntry {
    pub kind: Kind,
    /// Index (staged) side of the `XY` field.
    pub index: Change,
    /// Worktree (unstaged) side of the `XY` field.
    pub worktree: Change,
    pub path: String,
    /// Original path for rename/copy entries.
    pub original_path: Option<String>,
    /// Similarity/score for rename/copy entries (0-100), if present.
    pub score: Option<i64>,
}

impl GitFileEntry {
    pub fn new(kind: Kind, path: impl Into<String>) -> Self {
        Self {
            kind,
            index: Change::Unmodified,
            worktree: Change::Unmodified,
            path: path.into(),
            original_path: None,
            score: None,
        }
    }

    /// Ordinary/renamed-or-copied entries are staged when their index side
    /// changed. Unmerged/untracked/ignored entries are never "staged"
    /// (unmerged conflicts show up as unstaged instead — see `is_unstaged`).
    pub fn is_staged(&self) -> bool {
        match self.kind {
            Kind::Ordinary | Kind::RenamedOrCopied => self.index != Change::Unmodified,
            _ => false,
        }
    }

    /// Ordinary/renamed-or-copied entries are unstaged when their worktree
    /// side changed. Unmerged entries are *always* unstaged (regardless of
    /// their XY code), matching the Swift source's unconditional `true`.
    pub fn is_unstaged(&self) -> bool {
        match self.kind {
            Kind::Ordinary | Kind::RenamedOrCopied => self.worktree != Change::Unmodified,
            Kind::Unmerged => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ordinary,
    RenamedOrCopied,
    Unmerged,
    Untracked,
    Ignored,
}

/// Status code for one side of an `XY` pair. `Unmodified` is porcelain's `.`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Unmodified,
    Modified,
    TypeChanged,
    Added,
    Deleted,
    Renamed,
    Copied,
    UpdatedButUnmerged,
}

impl Change {
    /// Any character not among the recognised porcelain codes falls back to
    /// `Unmodified`, mirroring the Swift `Change(porcelain:)` initializer's
    /// `?? .unmodified` fallback (no error, no panic).
    pub fn from_porcelain(c: char) -> Change {
        match c {
            '.' => Change::Unmodified,
            'M' => Change::Modified,
            'T' => Change::TypeChanged,
            'A' => Change::Added,
            'D' => Change::Deleted,
            'R' => Change::Renamed,
            'C' => Change::Copied,
            'U' => Change::UpdatedButUnmerged,
            _ => Change::Unmodified,
        }
    }

    /// Inverse of `from_porcelain`; the single-character porcelain code.
    pub fn to_porcelain(self) -> char {
        match self {
            Change::Unmodified => '.',
            Change::Modified => 'M',
            Change::TypeChanged => 'T',
            Change::Added => 'A',
            Change::Deleted => 'D',
            Change::Renamed => 'R',
            Change::Copied => 'C',
            Change::UpdatedButUnmerged => 'U',
        }
    }
}
