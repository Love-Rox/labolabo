//! Golden-oracle test for `tiling::TileLayout`/`PanePayload`, anchored to
//! real `JSONEncoder().encode(_:)` output from the Swift app's
//! `app/Sources/PaneTilingModel.swift` -- see `src/tiling.rs`'s module doc
//! comment for the full compatibility contract (key spellings, omission
//! rules, `/`-escaping, float formatting) and, importantly, *why* matching
//! Swift's object key **order** is neither attempted nor meaningful
//! (empirically, `JSONEncoder`'s key order isn't even stable across
//! repeated runs of the same Swift process).
//!
//! `fixtures/tiling/*.json` (except `legacy_old_format.json`, see below)
//! were produced by a disposable, not-checked-in Swift oracle script that
//! compiles a small driver together with the real
//! `app/Sources/PaneTilingModel.swift` and runs `JSONEncoder().encode(_:)`
//! over representative `TileLayout` values -- a single leaf, a single leaf
//! with an agent session, a tab group, a 3-level-deep split (the real
//! `defaultLayout()` shape), and a leaf exercising string escaping
//! (forward slashes, quotes, backslash, control chars, emoji, Japanese).
//! `legacy_old_format.json` is hand-authored instead: it represents
//! genuinely pre-tab-feature persisted data (only `paneKind`/`paneTitle`,
//! no agent-session keys at all, since those didn't exist yet either),
//! which by definition can't come from *today's* `PaneTilingModel.swift`.
//!
//! To regenerate the Swift-oracle-produced fixtures (e.g. after changing
//! `TileLayout`'s shape), from the repo root:
//!
//! ```sh
//! cat > /tmp/main.swift <<'EOF'
//! import Foundation
//! func write(_ name: String, _ layout: TileLayout, dir: String) {
//!     let data = try! JSONEncoder().encode(layout)
//!     let json = String(data: data, encoding: .utf8)!
//!     try! json.write(
//!         toFile: (dir as NSString).appendingPathComponent("\(name).json"),
//!         atomically: true, encoding: .utf8)
//! }
//! let outDir = CommandLine.arguments[1]
//! write("single_leaf", TileLayout(paneKind: "terminal", paneTitle: "端末"), dir: outDir)
//! // ... one `write(...)` call per fixture; see this file's per-fixture
//! // assertions below for the exact shape each fixture must have.
//! EOF
//! swiftc -O /tmp/main.swift app/Sources/PaneTilingModel.swift -o /tmp/tiling_fixtures_gen
//! /tmp/tiling_fixtures_gen rust/crates/labolabo-core/fixtures/tiling
//! ```
//!
//! `swiftc` requires the file with top-level statements to be literally
//! named `main.swift`; `PaneTilingModel.swift` contributes only
//! declarations (`TileLayout`/`PanePayload` don't touch `@MainActor`/
//! `Observation`, so this compiles standalone without pulling in AppKit).
//! This is a *separate* mechanism from `fixtures/generate.swift` (which
//! links against pre-built `LaboLaboEngine` object files) because
//! `PaneTilingModel.swift` lives in the app target, not the
//! `LaboLaboEngine` SwiftPM library -- `generate.swift`'s linking trick
//! can't reach it.
//!
//! ## What "golden" means here
//!
//! For every fixture this test:
//!
//! 1. **Decodes** the real Swift-produced JSON and asserts the resulting
//!    tree has the exact expected shape -- the primary compatibility
//!    contract (existing users' persisted layouts must load correctly).
//! 2. **Round-trips**: decode -> `PaneTilingModel` -> `snapshot()` ->
//!    decode again -> asserts the two decoded trees are equal. This proves
//!    the encode/decode pair is lossless, which is the substantive
//!    property "byte-identical round trip" is a proxy for (see the caveat
//!    above about why *literal* byte-identity to Swift's output isn't a
//!    coherent target for key order).
//! 3. **Cross-checks JSON *content* equivalence**: parses both the
//!    original Swift-produced JSON and this crate's re-encoded JSON as
//!    `serde_json::Value` and asserts they're equal. `Value`'s `Object` is
//!    a `Map` (order-independent equality) and JSON parsing normalizes
//!    `\/` back to `/`, so this is a genuine "same JSON document"
//!    assertion, order and escaping-style aside -- and it's exactly what
//!    would fail if the `/`-escaping or float-formatting `Formatter`
//!    customizations in `tiling::swift_json` (or a key-spelling typo) were
//!    wrong, since a mismatched number spelling (`"1"` vs `"1.0"`) parses
//!    to a different `serde_json::Number` variant.
//! 4. **Confirms this crate's own encoder is stable**: encoding the same
//!    value twice produces byte-identical output both times -- a property
//!    Swift's `JSONEncoder` itself does not have (see `src/tiling.rs`).

use labolabo_core::{PaneKind, PaneTilingModel, TileLayout, TileOrientation};
use std::fs;
use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/tiling")
}

fn read_fixture(name: &str) -> String {
    let path = fixtures_dir().join(format!("{name}.json"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// Common round-trip + content-equivalence + self-stability checks shared by
/// every fixture (see the module doc comment, points 2-4). Point 1
/// (shape-specific decode assertions) lives in each `#[test]` below.
fn assert_golden_round_trip(fixture_name: &str) {
    let raw = read_fixture(fixture_name);

    let decoded = TileLayout::from_json(&raw).unwrap_or_else(|e| {
        panic!("fixture `{fixture_name}` failed to parse as TileLayout JSON: {e}")
    });
    let model = PaneTilingModel::model_from(&decoded).unwrap_or_else(|| {
        panic!("fixture `{fixture_name}` decoded to TileLayout but not to a valid TileNode tree")
    });

    // (2) decode -> encode -> decode is lossless.
    let re_encoded = model.snapshot();
    let re_decoded_model = PaneTilingModel::model_from(&re_encoded).unwrap_or_else(|| {
        panic!("fixture `{fixture_name}`: re-encoded snapshot failed to decode")
    });
    assert_eq!(
        re_encoded,
        re_decoded_model.snapshot(),
        "fixture `{fixture_name}`: decode(encode(decode(x))) should equal encode(decode(x))"
    );
    assert_eq!(
        model.panes().iter().map(|p| p.kind).collect::<Vec<_>>(),
        re_decoded_model
            .panes()
            .iter()
            .map(|p| p.kind)
            .collect::<Vec<_>>(),
        "fixture `{fixture_name}`: pane kinds should survive an encode/decode round trip"
    );

    // (3) same JSON *content* as the original Swift-produced fixture,
    // order/escaping-style aside.
    let original_value: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("fixture `{fixture_name}` is not valid JSON: {e}"));
    let rust_json = decoded.to_json();
    let rust_value: serde_json::Value = serde_json::from_str(&rust_json).unwrap_or_else(|e| {
        panic!("fixture `{fixture_name}`: this crate's own to_json() output didn't parse: {e}")
    });
    assert_eq!(
        original_value, rust_value,
        "fixture `{fixture_name}`: re-encoded JSON should be the same document as the Swift-produced original \
         (same keys/omissions/values), regardless of key order.\n  original: {raw}\n  rust:     {rust_json}"
    );

    // (4) this crate's encoder is deterministic across repeated calls
    // (unlike `JSONEncoder`'s key order -- see the module doc comment).
    assert_eq!(
        decoded.to_json(),
        decoded.to_json(),
        "fixture `{fixture_name}`: to_json() should be stable across repeated calls"
    );
}

#[test]
fn single_leaf() {
    assert_golden_round_trip("single_leaf");

    let layout = TileLayout::from_json(&read_fixture("single_leaf")).unwrap();
    assert_eq!(layout.pane_kind.as_deref(), Some("terminal"));
    assert_eq!(layout.pane_title.as_deref(), Some("端末"));
    assert!(layout.pane_agent_session_id.is_none());
    assert!(layout.panes.is_none());

    let model = PaneTilingModel::model_from(&layout).unwrap();
    assert!(model.root.is_leaf());
    assert_eq!(model.root.panes.len(), 1);
    assert_eq!(model.root.panes[0].kind, PaneKind::Terminal);
    assert_eq!(model.root.panes[0].title, "端末");
    assert!(model.root.panes[0].agent_session_id.is_none());
}

#[test]
fn single_leaf_with_agent_session() {
    assert_golden_round_trip("single_leaf_with_agent_session");

    let layout = TileLayout::from_json(&read_fixture("single_leaf_with_agent_session")).unwrap();
    let model = PaneTilingModel::model_from(&layout).unwrap();
    let pane = &model.root.panes[0];
    assert_eq!(pane.kind, PaneKind::Terminal);
    assert_eq!(pane.title, "Claude");
    assert_eq!(
        pane.agent_session_id.as_deref(),
        Some("5f2c1e2a-...-session")
    );
    assert_eq!(
        pane.agent_transcript_path.as_deref(),
        Some("/Users/me/.claude/projects/foo/5f2c1e2a.jsonl"),
        "decoded transcript path should have Swift's `\\/` un-escaped back to `/`"
    );
}

#[test]
fn tab_group() {
    assert_golden_round_trip("tab_group");

    let layout = TileLayout::from_json(&read_fixture("tab_group")).unwrap();
    assert!(
        layout.panes.is_some(),
        "2+ tabs should use the panes/selectedIndex shape, not legacy paneKind"
    );
    let model = PaneTilingModel::model_from(&layout).unwrap();
    assert!(model.root.is_leaf());
    assert_eq!(model.root.panes.len(), 2);
    assert_eq!(model.root.selected_index, 1);
    assert_eq!(model.root.panes[0].title, "t1");
    assert_eq!(
        model.root.panes[0].agent_session_id.as_deref(),
        Some("sid-1")
    );
    assert_eq!(
        model.root.panes[0].agent_transcript_path.as_deref(),
        Some("/tmp/t1.jsonl")
    );
    assert_eq!(model.root.panes[1].title, "t2");
    assert!(model.root.panes[1].agent_session_id.is_none());
}

#[test]
fn nested_split() {
    assert_golden_round_trip("nested_split");

    let layout = TileLayout::from_json(&read_fixture("nested_split")).unwrap();
    let model = PaneTilingModel::model_from(&layout).unwrap();

    // The real `defaultLayout()` shape: vertical root (0.55) over
    // terminal | horizontal row (0.25) of commits | (files:diff, 1/3).
    assert!(!model.root.is_leaf());
    assert_eq!(model.root.orientation, TileOrientation::Vertical);
    assert!((model.root.ratio - 0.55).abs() < 1e-9);
    assert_eq!(model.root.children.len(), 2);
    assert_eq!(
        model.root.children[0].selected_pane().map(|p| p.kind),
        Some(PaneKind::Terminal)
    );

    let bottom = &model.root.children[1];
    assert_eq!(bottom.orientation, TileOrientation::Horizontal);
    assert!((bottom.ratio - 0.25).abs() < 1e-9);
    assert_eq!(
        bottom.children[0].selected_pane().map(|p| p.kind),
        Some(PaneKind::Commits)
    );

    let files_and_diff = &bottom.children[1];
    assert_eq!(files_and_diff.orientation, TileOrientation::Horizontal);
    assert!(
        (files_and_diff.ratio - 1.0 / 3.0).abs() < 1e-9,
        "1/3 ratio should survive the JSON round trip at full float precision"
    );
    assert_eq!(
        files_and_diff.children[0].selected_pane().map(|p| p.kind),
        Some(PaneKind::Files)
    );
    assert_eq!(
        files_and_diff.children[1].selected_pane().map(|p| p.kind),
        Some(PaneKind::Diff)
    );

    assert_eq!(
        model.panes().iter().map(|p| p.kind).collect::<Vec<_>>(),
        vec![
            PaneKind::Terminal,
            PaneKind::Commits,
            PaneKind::Files,
            PaneKind::Diff
        ]
    );
}

#[test]
fn unicode_and_special_chars() {
    assert_golden_round_trip("unicode_and_special_chars");

    let layout = TileLayout::from_json(&read_fixture("unicode_and_special_chars")).unwrap();
    let model = PaneTilingModel::model_from(&layout).unwrap();
    assert_eq!(
        model.root.panes[0].title,
        "a/b \"quoted\" \\slash\\ line1\nline2\ttab 😀 タブ",
        "forward slashes, quotes, backslashes, control chars, emoji, and Japanese should all decode intact"
    );
    assert_eq!(
        model.root.panes[0].agent_transcript_path.as_deref(),
        Some("/Users/me/repo/a b/transcript.jsonl")
    );
}

#[test]
fn legacy_old_format() {
    assert_golden_round_trip("legacy_old_format");

    let layout = TileLayout::from_json(&read_fixture("legacy_old_format")).unwrap();
    let model = PaneTilingModel::model_from(&layout).unwrap();
    assert!(model.root.is_leaf());
    assert_eq!(model.root.panes.len(), 1);
    assert_eq!(model.root.panes[0].kind, PaneKind::Files);
    assert_eq!(model.root.panes[0].title, "変更ファイル");
    assert!(model.root.panes[0].agent_session_id.is_none());
    assert!(model.root.panes[0].agent_transcript_path.is_none());
}

#[test]
fn all_fixtures_are_covered() {
    let mut found: Vec<String> = fs::read_dir(fixtures_dir())
        .expect("fixtures/tiling should exist")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .map(|p| p.file_stem().unwrap().to_str().unwrap().to_string())
        .collect();
    found.sort();
    let mut expected = vec![
        "legacy_old_format",
        "nested_split",
        "single_leaf",
        "single_leaf_with_agent_session",
        "tab_group",
        "unicode_and_special_chars",
    ];
    expected.sort();
    assert_eq!(
        found, expected,
        "a fixture was added/removed without updating this list (and presumably without adding a matching #[test])"
    );
}
