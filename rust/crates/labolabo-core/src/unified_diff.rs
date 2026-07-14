//! Faithful port of `Sources/LaboLaboEngine/Git/UnifiedDiffParser.swift`.
//!
//! Parser for unified `git diff` / `git diff --cached` output (possibly
//! multi-file).
//!
//! IMPORTANT (preserved, not "fixed"): every line-prefix check below (for
//! `"diff --git "`, `"--- "`, `"+++ "`, `"new file mode"`, `"rename from "`,
//! `"@@"`, etc.) runs *unconditionally* against every line, including lines
//! inside an already-open hunk. The Swift source does not gate these checks
//! on parser state. This means hunk *content* that happens to start with one
//! of these literal prefixes gets mis-classified as file-header metadata
//! instead of a hunk line. See the `quirk_*` test below and
//! `fixtures/inputs/diff/quirk_dash_dash_dash_deletion_line.diff` for a
//! concrete, real case: a deleted line whose text begins with `"-- "`
//! renders as `"--- ..."` and is swallowed as a bogus `oldPath`.

use crate::util::{drop_first_chars, split_space_omitting_empty};

/// One file's worth of parsed `git diff` output.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileDiff {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub is_binary: bool,
    pub is_new: bool,
    pub is_deleted: bool,
    pub is_rename: bool,
    pub hunks: Vec<DiffHunk>,
}

impl FileDiff {
    pub fn display_path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or("")
    }

    pub fn additions(&self) -> i64 {
        self.hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| l.kind == LineKind::Addition)
            .count() as i64
    }

    pub fn deletions(&self) -> i64 {
        self.hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| l.kind == LineKind::Deletion)
            .count() as i64
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    pub header: String,
    pub old_start: i64,
    pub old_count: i64,
    pub new_start: i64,
    pub new_count: i64,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
    pub old_line_number: Option<i64>,
    pub new_line_number: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Addition,
    Deletion,
    NoNewline,
}

pub fn parse(raw: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut current: Option<FileDiff> = None;
    let mut hunk: Option<DiffHunk> = None;
    let mut old_line: i64 = 0;
    let mut new_line: i64 = 0;

    // `split('\n')` (not `split_terminator`) intentionally keeps a trailing
    // empty element when `raw` ends in "\n", mirroring Swift's
    // `split(separator: "\n", omittingEmptySubsequences: false)`.
    for line in raw.split('\n') {
        if line.starts_with("diff --git ") {
            flush_file(&mut files, &mut current, &mut hunk);
            current = Some(FileDiff::default());
        } else if line.starts_with("--- ") {
            if let Some(c) = current.as_mut() {
                c.old_path = path_from(line, "--- ");
            }
        } else if line.starts_with("+++ ") {
            if let Some(c) = current.as_mut() {
                c.new_path = path_from(line, "+++ ");
            }
        } else if line.starts_with("new file mode") {
            if let Some(c) = current.as_mut() {
                c.is_new = true;
            }
        } else if line.starts_with("deleted file mode") {
            if let Some(c) = current.as_mut() {
                c.is_deleted = true;
            }
        } else if line.starts_with("rename from ") {
            if let Some(c) = current.as_mut() {
                c.is_rename = true;
                c.old_path =
                    Some(drop_first_chars(line, "rename from ".chars().count()).to_string());
            }
        } else if line.starts_with("rename to ") {
            if let Some(c) = current.as_mut() {
                c.is_rename = true;
                c.new_path = Some(drop_first_chars(line, "rename to ".chars().count()).to_string());
            }
        } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            if let Some(c) = current.as_mut() {
                c.is_binary = true;
            }
        } else if line.starts_with("@@") {
            flush_hunk(&mut current, &mut hunk);
            let (old_start, old_count, new_start, new_count) = parse_hunk_header(line);
            hunk = Some(DiffHunk {
                header: line.to_string(),
                old_start,
                old_count,
                new_start,
                new_count,
                lines: Vec::new(),
            });
            old_line = old_start;
            new_line = new_start;
        } else if let Some(h) = hunk.as_mut() {
            match line.chars().next() {
                Some('+') => {
                    h.lines.push(DiffLine {
                        kind: LineKind::Addition,
                        text: drop_first_chars(line, 1).to_string(),
                        old_line_number: None,
                        new_line_number: Some(new_line),
                    });
                    new_line += 1;
                }
                Some('-') => {
                    h.lines.push(DiffLine {
                        kind: LineKind::Deletion,
                        text: drop_first_chars(line, 1).to_string(),
                        old_line_number: Some(old_line),
                        new_line_number: None,
                    });
                    old_line += 1;
                }
                Some(' ') => {
                    h.lines.push(DiffLine {
                        kind: LineKind::Context,
                        text: drop_first_chars(line, 1).to_string(),
                        old_line_number: Some(old_line),
                        new_line_number: Some(new_line),
                    });
                    old_line += 1;
                    new_line += 1;
                }
                Some('\\') => {
                    // "\ No newline at end of file"
                    h.lines.push(DiffLine {
                        kind: LineKind::NoNewline,
                        text: drop_first_chars(line, 2).to_string(),
                        old_line_number: None,
                        new_line_number: None,
                    });
                }
                _ => {
                    // blank trailing artifacts / unknown markers: ignored.
                }
            }
        }
    }
    flush_file(&mut files, &mut current, &mut hunk);
    files
}

fn flush_hunk(current: &mut Option<FileDiff>, hunk: &mut Option<DiffHunk>) {
    if let Some(h) = hunk.take() {
        if let Some(c) = current.as_mut() {
            c.hunks.push(h);
        }
    }
}

fn flush_file(
    files: &mut Vec<FileDiff>,
    current: &mut Option<FileDiff>,
    hunk: &mut Option<DiffHunk>,
) {
    flush_hunk(current, hunk);
    if let Some(c) = current.take() {
        files.push(c);
    }
}

// MARK: - Helpers

fn path_from(line: &str, prefix: &str) -> Option<String> {
    let mut p = drop_first_chars(line, prefix.chars().count());
    if let Some(tab_idx) = p.find('\t') {
        p = &p[..tab_idx];
    }
    if p == "/dev/null" {
        return None;
    }
    if p.starts_with("a/") || p.starts_with("b/") {
        p = drop_first_chars(p, 2);
    }
    Some(p.to_string())
}

fn parse_hunk_header(line: &str) -> (i64, i64, i64, i64) {
    let comps = split_space_omitting_empty(line);
    if comps.len() < 3 {
        return (0, 1, 0, 1);
    }
    let old = parse_range(drop_first_chars(comps[1], 1)); // strip '-'
    let new = parse_range(drop_first_chars(comps[2], 1)); // strip '+'
    (old.0, old.1, new.0, new.1)
}

fn parse_range(s: &str) -> (i64, i64) {
    let parts: Vec<&str> = s.split(',').filter(|p| !p.is_empty()).collect();
    let start = if parts.is_empty() {
        0
    } else {
        parts[0].parse().unwrap_or(0)
    };
    let count = if parts.len() > 1 {
        parts[1].parse().unwrap_or(1)
    } else {
        1
    };
    (start, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Joins lines with "\n" (no trailing newline), matching how the ported
    /// Swift XCTest cases embed diff text via multiline string literals.
    /// NOTE: deliberately not using Rust's `"...\n\` line-continuation
    /// trick here -- it silently strips leading whitespace on the
    /// continued line, which eats the leading-space *context-line* marker
    /// diff lines rely on.
    fn lines(items: &[&str]) -> String {
        items.join("\n")
    }

    // Ported 1:1 from Tests/LaboLaboEngineTests/UnifiedDiffParserTests.swift.

    #[test]
    fn modified_file_hunk_and_line_numbers() {
        let raw = lines(&[
            "diff --git a/src/foo.swift b/src/foo.swift",
            "index 1111111..2222222 100644",
            "--- a/src/foo.swift",
            "+++ b/src/foo.swift",
            "@@ -1,3 +1,4 @@",
            " line1",
            "-line2",
            "+line2 changed",
            "+line3 added",
            " line4",
        ]);
        let files = parse(&raw);
        assert_eq!(files.len(), 1);

        let file = &files[0];
        assert_eq!(file.old_path.as_deref(), Some("src/foo.swift"));
        assert_eq!(file.new_path.as_deref(), Some("src/foo.swift"));
        assert_eq!(file.display_path(), "src/foo.swift");
        assert!(!file.is_binary);
        assert_eq!(file.additions(), 2);
        assert_eq!(file.deletions(), 1);

        assert_eq!(file.hunks.len(), 1);
        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.old_count, 3);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_count, 4);
        assert_eq!(
            hunk.lines.iter().map(|l| l.kind).collect::<Vec<_>>(),
            vec![
                LineKind::Context,
                LineKind::Deletion,
                LineKind::Addition,
                LineKind::Addition,
                LineKind::Context
            ]
        );

        // Line numbering: context keeps both, addition only new, deletion only old.
        assert_eq!(hunk.lines[0].old_line_number, Some(1));
        assert_eq!(hunk.lines[0].new_line_number, Some(1));
        assert_eq!(hunk.lines[1].old_line_number, Some(2));
        assert_eq!(hunk.lines[1].new_line_number, None);
        assert_eq!(hunk.lines[2].new_line_number, Some(2));
        assert_eq!(hunk.lines[2].old_line_number, None);
        assert_eq!(hunk.lines[4].old_line_number, Some(3));
        assert_eq!(hunk.lines[4].new_line_number, Some(4));
    }

    #[test]
    fn new_file() {
        let raw = lines(&[
            "diff --git a/new.txt b/new.txt",
            "new file mode 100644",
            "index 0000000..abc1234",
            "--- /dev/null",
            "+++ b/new.txt",
            "@@ -0,0 +1,2 @@",
            "+hello",
            "+world",
        ]);
        let files = parse(&raw);
        assert_eq!(files.len(), 1);
        let file = &files[0];
        assert!(file.is_new);
        assert_eq!(file.old_path, None, "/dev/null maps to nil");
        assert_eq!(file.new_path.as_deref(), Some("new.txt"));
        assert_eq!(file.additions(), 2);
        assert_eq!(file.deletions(), 0);
    }

    #[test]
    fn binary_file() {
        let raw = lines(&[
            "diff --git a/img.png b/img.png",
            "index aaaaaaa..bbbbbbb 100644",
            "Binary files a/img.png and b/img.png differ",
        ]);
        let files = parse(&raw);
        assert_eq!(files.len(), 1);
        assert!(files[0].is_binary);
        assert!(files[0].hunks.is_empty());
    }

    #[test]
    fn multiple_files() {
        let raw = lines(&[
            "diff --git a/a.txt b/a.txt",
            "--- a/a.txt",
            "+++ b/a.txt",
            "@@ -1 +1 @@",
            "-old",
            "+new",
            "diff --git a/b.txt b/b.txt",
            "--- a/b.txt",
            "+++ b/b.txt",
            "@@ -1,2 +1,2 @@",
            " keep",
            "-drop",
            "+add",
        ]);
        let files = parse(&raw);
        assert_eq!(
            files
                .iter()
                .map(|f| f.display_path().to_string())
                .collect::<Vec<_>>(),
            vec!["a.txt", "b.txt"]
        );
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[1].hunks.len(), 1);
    }

    // Additional edge cases not covered by the existing Swift test suite but
    // exercised by the shared golden fixtures too (see fixtures/inputs/diff).

    #[test]
    fn empty_input_yields_no_files() {
        assert_eq!(parse(""), Vec::new());
    }

    #[test]
    fn deleted_file() {
        let raw = lines(&[
            "diff --git a/gone.txt b/gone.txt",
            "deleted file mode 100644",
            "index abc1234..0000000",
            "--- a/gone.txt",
            "+++ /dev/null",
            "@@ -1,2 +0,0 @@",
            "-bye",
            "-cruel world",
        ]);
        let files = parse(&raw);
        assert_eq!(files.len(), 1);
        let file = &files[0];
        assert!(file.is_deleted);
        assert_eq!(file.old_path.as_deref(), Some("gone.txt"));
        assert_eq!(file.new_path, None);
        assert_eq!(file.deletions(), 2);
    }

    #[test]
    fn rename_with_hunk_sets_is_rename_and_parses_content() {
        let raw = lines(&[
            "diff --git a/old/name.txt b/new/name.txt",
            "similarity index 90%",
            "rename from old/name.txt",
            "rename to new/name.txt",
            "index 1111111..2222222 100644",
            "--- a/old/name.txt",
            "+++ b/new/name.txt",
            "@@ -1,2 +1,2 @@",
            " unchanged",
            "-old text",
            "+new text",
        ]);
        let files = parse(&raw);
        assert_eq!(files.len(), 1);
        let file = &files[0];
        assert!(file.is_rename);
        assert_eq!(file.old_path.as_deref(), Some("old/name.txt"));
        assert_eq!(file.new_path.as_deref(), Some("new/name.txt"));
        assert_eq!(file.hunks.len(), 1);
    }

    #[test]
    fn pure_rename_without_content_change_has_no_hunks_but_sets_paths() {
        // Mirrors what `git diff --cached -M` emits for a 100%-similarity
        // rename: only "rename from"/"rename to" lines, no "--- "/"+++ "
        // or hunk at all.
        let raw = lines(&[
            "diff --git a/src/old_name.txt b/src/new_name.txt",
            "similarity index 100%",
            "rename from src/old_name.txt",
            "rename to src/new_name.txt",
        ]);
        let files = parse(&raw);
        assert_eq!(files.len(), 1);
        let file = &files[0];
        assert!(file.is_rename);
        assert_eq!(file.old_path.as_deref(), Some("src/old_name.txt"));
        assert_eq!(file.new_path.as_deref(), Some("src/new_name.txt"));
        assert!(file.hunks.is_empty());
    }

    #[test]
    fn no_newline_at_eof_marker_has_no_line_numbers() {
        let raw = lines(&[
            "diff --git a/tail.txt b/tail.txt",
            "index 1111111..2222222 100644",
            "--- a/tail.txt",
            "+++ b/tail.txt",
            "@@ -1,2 +1,2 @@",
            " keep",
            "-old tail",
            "\\ No newline at end of file",
            "+new tail",
            "\\ No newline at end of file",
        ]);
        let files = parse(&raw);
        let hunk = &files[0].hunks[0];
        // context(keep), deletion(old tail), noNewline, addition(new tail), noNewline
        assert_eq!(hunk.lines.len(), 5);
        assert_eq!(hunk.lines[2].kind, LineKind::NoNewline);
        assert_eq!(hunk.lines[2].text, "No newline at end of file");
        assert_eq!(hunk.lines[2].old_line_number, None);
        assert_eq!(hunk.lines[2].new_line_number, None);
        assert_eq!(hunk.lines[4].kind, LineKind::NoNewline);
    }

    #[test]
    fn multiple_hunks_in_a_single_file() {
        let raw = lines(&[
            "diff --git a/multi.txt b/multi.txt",
            "index 1111111..2222222 100644",
            "--- a/multi.txt",
            "+++ b/multi.txt",
            "@@ -1,2 +1,2 @@",
            " top",
            "-first",
            "+first changed",
            "@@ -10,2 +10,3 @@",
            " bottom",
            "+extra",
            " end",
        ]);
        let files = parse(&raw);
        assert_eq!(files[0].hunks.len(), 2);
        assert_eq!(files[0].hunks[1].old_start, 10);
    }

    #[test]
    fn quirk_deletion_line_starting_with_dash_dash_dash_is_misparsed_as_old_path_header() {
        // Documents (does not "fix") a real Swift-implementation quirk: the
        // "--- " prefix check runs unconditionally against every line, even
        // inside an open hunk. A deleted content line whose text begins with
        // "-- " renders as a raw diff line starting with "--- ", so it gets
        // consumed as `oldPath` instead of appended to the hunk.
        let raw = lines(&[
            "diff --git a/quirk.txt b/quirk.txt",
            "index 1111111..2222222 100644",
            "--- a/quirk.txt",
            "+++ b/quirk.txt",
            "@@ -1,3 +1,2 @@",
            " keep",
            "--- looks like a header but is really a deleted line",
            " tail",
        ]);
        let files = parse(&raw);
        assert_eq!(files.len(), 1);
        let file = &files[0];
        // oldPath got clobbered by the quirky line instead of staying "quirk.txt".
        assert_eq!(
            file.old_path.as_deref(),
            Some("looks like a header but is really a deleted line")
        );
        assert_eq!(file.new_path.as_deref(), Some("quirk.txt"));
        // The deletion line itself never made it into the hunk.
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].lines.len(), 2);
        assert_eq!(
            file.hunks[0]
                .lines
                .iter()
                .map(|l| l.kind)
                .collect::<Vec<_>>(),
            vec![LineKind::Context, LineKind::Context]
        );
        assert_eq!(file.deletions(), 0);
    }
}
