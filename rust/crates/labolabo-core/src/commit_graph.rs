//! Faithful port of the pure layout algorithm in
//! `Sources/LaboLaboEngine/Git/CommitGraph.swift`.
//!
//! Only `CommitGraphLayout.build(_:)` (a pure function from raw `git log`
//! output to laid-out rows) and its result types are ported. The Swift
//! file's `GitEngine.commitGraph(worktree:limit:)` extension — which shells
//! out to `git log` via `GitRunner` — is process execution, not pure logic,
//! and is out of scope for this crate (see the crate's module doc comment).
//!
//! Lanes are **stable columns**: a branch keeps its column until it merges,
//! so passing lanes are drawn straight and only real branch/merge rows bend
//! (unlike `git log --graph`'s ASCII art, which shifts lanes left whenever a
//! column closes). Layout is computed from each commit's parents (`%P`).

use crate::util::split_space_omitting_empty;

/// One row of the commit graph = exactly one commit, plus the lane layout
/// needed to draw its node and the edges crossing that row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitGraphRow {
    pub id: usize,
    pub commit: Commit,
    /// Column of this commit's node.
    pub node_lane: usize,
    /// Edges crossing this row (passing lanes + connections to the node).
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub author: String,
    /// Author date as Unix epoch seconds (from `%at`). The Swift source
    /// exposes a Foundation `Date` here; this crate has no date/time
    /// dependency (matching wave 1's "no runtime deps beyond what's
    /// strictly needed" stance), so the raw epoch seconds are kept instead
    /// — relative/absolute display formatting is a UI-layer concern on both
    /// sides anyway.
    pub date: Option<i64>,
    /// Decorations like `HEAD -> main, origin/main` (parens stripped); empty if none.
    pub refs: String,
}

/// A single edge segment within a row.
/// - `Through`: a lane passing straight through this row (top→bottom, same column).
/// - `NodeIn`: a child line entering the node from above (its column → node, top half).
/// - `NodeOut`: a parent line leaving the node downward (node → its column, bottom half).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeShape {
    Through,
    NodeIn,
    NodeOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub shape: EdgeShape,
    /// The column this edge attaches to (the non-node end).
    pub lane: usize,
    /// Which lane index determines the color (kept == `lane` for stable coloring).
    pub color_lane: usize,
}

const UNIT_SEPARATOR: char = '\u{1f}';

struct RawCommit {
    full: String,
    short: String,
    subject: String,
    author: String,
    date: Option<i64>,
    parents: Vec<String>,
    refs: String,
}

fn parse_raw_commits(raw: &str) -> Vec<RawCommit> {
    let mut commits = Vec::new();
    // `split(separator: "\n", omittingEmptySubsequences: true)`: blank lines dropped.
    for raw_line in raw.split('\n').filter(|l| !l.is_empty()) {
        let parts: Vec<&str> = raw_line.split(UNIT_SEPARATOR).collect();
        if parts.len() < 6 {
            continue;
        }
        let part = |i: usize| -> &str { parts.get(i).copied().unwrap_or("") };
        let parents = split_space_omitting_empty(part(5))
            .into_iter()
            .map(String::from)
            .collect();
        let date = part(4).parse::<i64>().ok();
        let refs = part(6)
            .trim_matches(|c| c == ' ' || c == '(' || c == ')')
            .to_string();
        commits.push(RawCommit {
            full: part(0).to_string(),
            short: part(1).to_string(),
            subject: part(2).to_string(),
            author: part(3).to_string(),
            date,
            parents,
            refs,
        });
    }
    commits
}

fn first_free(lanes: &mut Vec<Option<String>>) -> usize {
    if let Some(i) = lanes.iter().position(|l| l.is_none()) {
        return i;
    }
    lanes.push(None);
    lanes.len() - 1
}

/// Builds the laid-out commit graph rows from raw `git log` output in the
/// `%H<US>%h<US>%s<US>%an<US>%at<US>%P<US>%d` line format (`<US>` = `\u{1f}`).
pub fn build(raw: &str) -> Vec<CommitGraphRow> {
    let commits = parse_raw_commits(raw);

    // lanes[i] = the (full) hash the lane is currently waiting to reach, or None if free.
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows: Vec<CommitGraphRow> = Vec::new();

    for (idx, c) in commits.iter().enumerate() {
        let mut edges: Vec<Edge> = Vec::new();

        // Children (already-drawn commits above) whose lane awaits this commit.
        let my_cols: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.as_deref() == Some(c.full.as_str()))
            .map(|(i, _)| i)
            .collect();
        let node_lane = my_cols
            .iter()
            .copied()
            .min()
            .unwrap_or_else(|| first_free(&mut lanes));

        // Lanes not involved with this node pass straight through.
        for (i, lane) in lanes.iter().enumerate() {
            if lane.is_some() && !my_cols.contains(&i) {
                edges.push(Edge {
                    shape: EdgeShape::Through,
                    lane: i,
                    color_lane: i,
                });
            }
        }
        // Child lines converge into the node (top half).
        for &col in &my_cols {
            edges.push(Edge {
                shape: EdgeShape::NodeIn,
                lane: col,
                color_lane: col,
            });
        }
        // The node consumes those lanes; free them before assigning parents.
        for &col in &my_cols {
            lanes[col] = None;
        }

        if let Some(first) = c.parents.first() {
            // First parent continues straight down in the node's lane.
            lanes[node_lane] = Some(first.clone());
            edges.push(Edge {
                shape: EdgeShape::NodeOut,
                lane: node_lane,
                color_lane: node_lane,
            });
            // Additional parents (merge): reuse an existing lane if one already
            // awaits that parent, otherwise open a new lane.
            for parent in c.parents.iter().skip(1) {
                if let Some(existing) = lanes
                    .iter()
                    .position(|l| l.as_deref() == Some(parent.as_str()))
                {
                    edges.push(Edge {
                        shape: EdgeShape::NodeOut,
                        lane: existing,
                        color_lane: existing,
                    });
                } else {
                    let col = first_free(&mut lanes);
                    lanes[col] = Some(parent.clone());
                    edges.push(Edge {
                        shape: EdgeShape::NodeOut,
                        lane: col,
                        color_lane: col,
                    });
                }
            }
        } else if node_lane < lanes.len() {
            // Root commit: the lane ends here.
            lanes[node_lane] = None;
        }

        let commit = Commit {
            hash: c.short.clone(),
            subject: c.subject.clone(),
            author: c.author.clone(),
            date: c.date,
            refs: c.refs.clone(),
        };
        rows.push(CommitGraphRow {
            id: idx,
            commit,
            node_lane,
            edges,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported 1:1 from Tests/LaboLaboEngineTests/CommitGraphParserTests.swift.

    /// Build one raw log line in the `%H %h %s %an %at %P %d` layout.
    fn line(
        full: &str,
        short: &str,
        subject: &str,
        author: &str,
        at: i64,
        parents: &[&str],
        refs: &str,
    ) -> String {
        [
            full.to_string(),
            short.to_string(),
            subject.to_string(),
            author.to_string(),
            at.to_string(),
            parents.join(" "),
            refs.to_string(),
        ]
        .join(&UNIT_SEPARATOR.to_string())
    }

    fn shapes(row: &CommitGraphRow, shape: EdgeShape) -> Vec<usize> {
        let mut lanes: Vec<usize> = row
            .edges
            .iter()
            .filter(|e| e.shape == shape)
            .map(|e| e.lane)
            .collect();
        lanes.sort();
        lanes
    }

    #[test]
    fn parses_commit_fields() {
        let raw = line(
            "abc1234def",
            "abc1234",
            "feat: hello",
            "Alice",
            1_700_000_000,
            &[],
            " (HEAD -> main, origin/main)",
        );
        let rows = build(&raw);
        assert_eq!(rows.len(), 1);
        let c = &rows[0].commit;
        assert_eq!(c.hash, "abc1234");
        assert_eq!(c.subject, "feat: hello");
        assert_eq!(c.author, "Alice");
        assert_eq!(c.date, Some(1_700_000_000));
        assert_eq!(c.refs, "HEAD -> main, origin/main");
        // 親も子もない単独コミット: ノードは lane 0、エッジ無し。
        assert_eq!(rows[0].node_lane, 0);
        assert!(rows[0].edges.is_empty());
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let raw = [
            line("A", "A", "a", "X", 2, &["B"], ""),
            line("B", "B", "b", "X", 1, &[], ""),
        ]
        .join("\n");
        let rows = build(&raw);
        assert_eq!(rows.len(), 2);
        // A は最初の親 B へ下向き（nodeOut lane0）。
        assert_eq!(rows[0].node_lane, 0);
        assert_eq!(shapes(&rows[0], EdgeShape::NodeOut), vec![0]);
        // B は上から入るだけ（nodeIn lane0）。どちらも lane 0 の直線。
        assert_eq!(rows[1].node_lane, 0);
        assert_eq!(shapes(&rows[1], EdgeShape::NodeIn), vec![0]);
    }

    #[test]
    fn merge_uses_stable_lanes() {
        // m は親 a(第1)/b(第2) のマージ。b は a を親に持つ feature 1 コミット。
        let raw = [
            line("m", "m", "merge", "X", 3, &["a", "b"], ""),
            line("b", "b", "feat", "X", 2, &["a"], ""),
            line("a", "a", "init", "X", 1, &[], ""),
        ]
        .join("\n");
        let rows = build(&raw);
        assert_eq!(rows.len(), 3);

        // マージコミット: lane0=第1親 a、lane1=第2親 b へ 2 本の nodeOut。
        assert_eq!(rows[0].node_lane, 0);
        assert_eq!(shapes(&rows[0], EdgeShape::NodeOut), vec![0, 1]);

        // feature コミット b: lane0 は a を待って通過、自身は lane1。
        assert_eq!(rows[1].node_lane, 1);
        assert_eq!(shapes(&rows[1], EdgeShape::Through), vec![0]);
        assert_eq!(shapes(&rows[1], EdgeShape::NodeIn), vec![1]);
        assert_eq!(shapes(&rows[1], EdgeShape::NodeOut), vec![1]);

        // 分岐元 a: lane0 と lane1 の 2 本が合流（nodeIn 2 本）。
        assert_eq!(rows[2].node_lane, 0);
        assert_eq!(shapes(&rows[2], EdgeShape::NodeIn), vec![0, 1]);
    }
}
