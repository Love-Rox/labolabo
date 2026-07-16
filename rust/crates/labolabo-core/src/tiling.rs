//! Faithful port of `app/Sources/PaneTilingModel.swift` (the app target, not
//! `LaboLaboEngine`): the binary tile/tab tree that represents one session's
//! terminal + changed-files + diff + commit-history layout, and its
//! persisted-JSON shape.
//!
//! One session's workspace is a single binary tile tree. A leaf ([`TileNode`]
//! with `panes` non-empty) is a *tab group*: one or more [`PaneItem`]s bundled
//! as tabs, one of them selected. A split node has two children laid out
//! along an [`TileOrientation`] with a `ratio`.
//!
//! ## What's ported vs. deliberately redesigned
//!
//! The Swift source is `@MainActor @Observable` reference types
//! (`TileNode`/`PaneItem`/`PaneTilingModel` are classes) mutated in place
//! through live object references, with a `weak var coordinator:
//! PaneTilingActions?` for the one UI callback surface and an
//! `onLayoutChanged: (() -> Void)?` closure for persistence hooks. This port
//! keeps the *observable behavior* (every mutation, return value, and
//! `selectedIndex`/`revision` bookkeeping rule) identical but translates the
//! reference-type tree into an owned tree of plain structs (`Vec<TileNode>`
//! children, no parent pointers) mutated via recursive `&mut self` lookups —
//! idiomatic Rust for a strict (non-shared, non-cyclic) tree, and there is no
//! observation framework to port. `PaneTilingActions` becomes a `trait`
//! (per the porting brief). IDs: Swift uses `UUID` for both `TileNode.id`
//! and `PaneItem.id`; this port uses opaque incrementing [`NodeId`]/[`PaneId`]
//! newtypes instead (never serialized, never compared across the two
//! namespaces, so any uniquely-generated identity works — this avoids an
//! extra `uuid` dependency for a purely in-process-lifetime identity).
//!
//! `recordAgentSession(id:paneUUIDString:transcriptPath:)` parses a `UUID`
//! out of a `String` (because the hooks transport that calls it hands pane
//! IDs across a process boundary as JSON strings); [`PaneTilingModel::record_agent_session`]
//! takes a [`PaneId`] directly instead and leaves that string decoding to
//! whatever future Rust hooks-integration layer bridges the wire format to
//! this core (out of scope for this module, same as it's UI-independent
//! here).
//!
//! ## Serialization: `TileLayout`/`PanePayload`/`LayoutPreset`
//!
//! Unlike every other module in this crate, `TileLayout`/`PanePayload` are
//! not test-only JSON views: they are the app's actual `Codable` DTOs,
//! round-tripped through `JSONEncoder`/`JSONDecoder` to persist a session's
//! layout (GRDB `appState.paneLayout` column, see `SessionStore.encodeLayout`/
//! `decodeLayout`) and named layout presets. Existing users already have
//! layouts on disk in whatever exact shape Swift's `JSONEncoder` wrote them
//! in, so decoding must accept that shape exactly: same key spellings, same
//! "a single tab writes the legacy `paneKind`/`paneTitle` shape, 2+ tabs
//! write `panes`/`selectedIndex`" backward-compat rule, same
//! omitted-vs-`null` rule for absent optionals.
//!
//! **Key order is deliberately not matched**, and this needs justifying: a
//! `TileLayout` value encoded four times, in four separate `swift` process
//! invocations on this toolchain (Swift 6.3.3 / the open-source
//! swift-foundation `JSONEncoder`), produced **four different key orders**:
//!
//! ```text
//! {"paneTitle":"端末","paneAgentTranscriptPath":"...","paneKind":"terminal","paneAgentSessionId":"sid-1"}
//! {"paneAgentSessionId":"sid-1","paneKind":"terminal","paneTitle":"端末","paneAgentTranscriptPath":"..."}
//! {"paneKind":"terminal","paneAgentSessionId":"sid-1","paneTitle":"端末","paneAgentTranscriptPath":"..."}
//! {"paneKind":"terminal","paneAgentSessionId":"sid-1","paneAgentTranscriptPath":"...","paneTitle":"端末"}
//! ```
//!
//! `JSONEncoder`'s key order for a `Codable` struct comes from Swift's
//! per-process-random string hash seed, not field declaration order. Since
//! Swift itself cannot reproduce a stable key order across runs,
//! "byte-identical to what Swift wrote" is not a coherent target for key
//! order in the first place -- and JSON object semantics make it
//! unnecessary to chase: an object's key order carries no meaning, decoding
//! only needs the right key *names* (which `serde`'s derive handles in any
//! order natively), and this crate's `swift_json` formatter (below) instead
//! emits keys in `TileLayout`'s/`PanePayload`'s Rust field declaration
//! order, which -- unlike Swift's -- is at least stable across runs.
//!
//! What *is* matched byte-for-byte, because it's part of what a byte
//! genuinely persisted by the app looks like and *is* deterministic on the
//! Swift side: key spellings, the omitted-vs-`null` rule
//! (`#[serde(skip_serializing_if = "Option::is_none")]`, mirroring
//! `JSONEncoder`'s synthesized `encodeIfPresent` for `Optional` stored
//! properties), string escaping (`"`, `\`, control chars via named escapes
//! `\b \f \n \r \t` / `\u00XX`, **and `/` as `\/`** -- confirmed empirically;
//! `serde_json`'s default formatter does the first set but never escapes
//! `/`, since the JSON spec doesn't require it), and floating-point
//! rendering for `ratio` (`CGFloat`/`Double` via `JSONEncoder`: shortest
//! round-trip decimal, **no trailing `.0` for integral values** -- `1.0` ->
//! `"1"` -- confirmed empirically; `serde_json`'s default ryu-based
//! formatter always keeps a decimal point). See the `swift_json` module
//! below for the custom `serde_json::ser::Formatter` that reproduces the
//! `/`-escaping and float-formatting quirks (Rust's own `f64::to_string()`
//! already matches Swift's shortest-round-trip-no-trailing-zero behavior for
//! every `ratio` value exercised here, so no extra float-formatting crate is
//! needed).
//!
//! Golden fixtures for this compatibility contract live in
//! `fixtures/tiling/*.json` and are exercised by `tests/tiling_golden.rs`.
//! Unlike `fixtures/generate.swift` (which links against pre-built
//! `LaboLaboEngine` object files), those fixtures come from a *separate*,
//! disposable oracle script that compiles a tiny driver together with the
//! real `app/Sources/PaneTilingModel.swift` via `swiftc` -- `PaneTilingModel`
//! lives in the app target, not the `LaboLaboEngine` SwiftPM library, so it
//! isn't reachable through `generate.swift`'s linking trick. See
//! `tests/tiling_golden.rs`'s module doc comment for the exact regeneration
//! command; the script itself is not checked in (same "leaves no footprint"
//! policy as `generate.swift`'s documented fallback).

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

// MARK: - Opaque identities

fn next_id() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Identity of a [`PaneItem`] (one tab). Never serialized -- `TileLayout`
/// carries no ID at all, matching Swift (a `PaneItem`'s `UUID` is
/// runtime-only, rebuilt fresh on every decode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneId(u64);

impl PaneId {
    fn new() -> Self {
        PaneId(next_id())
    }
}

/// Identity of a [`TileNode`] (leaf or split). Distinct namespace from
/// [`PaneId`] purely for type-safety; the two are never compared against
/// each other, mirroring Swift (`TileNode.id` and `PaneItem.id` are both
/// `UUID` but semantically unrelated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u64);

impl NodeId {
    fn new() -> Self {
        NodeId(next_id())
    }
}

// MARK: - TileOrientation

/// Direction a split node lays its two children out along. Corresponds to
/// AppKit's `NSUserInterfaceLayoutOrientation` in the Swift source, kept
/// UI-independent here the same way the Swift model keeps it UI-independent
/// (the AppKit conversion lives at the UI layer's boundary, out of scope).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileOrientation {
    Horizontal,
    Vertical,
}

// MARK: - PaneKind

/// What a pane displays: a terminal surface, the changed-files list, a
/// diff, or the commit-history graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Terminal,
    Files,
    Diff,
    Commits,
}

impl PaneKind {
    /// Default title used when restoring a pane whose title was missing
    /// from persisted data. Not localized (this crate has no localization
    /// story yet); mirrors the base/Japanese strings `String(localized:)`
    /// resolves to in the Swift source today (there is no other locale
    /// catalog in the app currently).
    pub fn default_title(self) -> &'static str {
        match self {
            PaneKind::Terminal => "端末",
            PaneKind::Files => "変更ファイル",
            PaneKind::Diff => "Diff",
            PaneKind::Commits => "履歴",
        }
    }

    /// SF Symbol name shared by the pane header and tab chip. UI-only
    /// metadata that happens to live on the model in the Swift source; kept
    /// here for fidelity (it's a plain string constant, not an AppKit type).
    pub fn icon_name(self) -> &'static str {
        match self {
            PaneKind::Terminal => "terminal",
            PaneKind::Files => "list.bullet.rectangle",
            PaneKind::Diff => "doc.text",
            PaneKind::Commits => "point.3.connected.trianglepath.dotted",
        }
    }

    /// The `Codable`/persisted spelling. Stable -- it's a storage format key.
    pub fn raw_value(self) -> &'static str {
        match self {
            PaneKind::Terminal => "terminal",
            PaneKind::Files => "files",
            PaneKind::Diff => "diff",
            PaneKind::Commits => "commits",
        }
    }

    /// Inverse of [`PaneKind::raw_value`]; `None` for any unrecognized
    /// spelling (mirrors Swift's failable `PaneKind(rawValue:)`).
    pub fn from_raw_value(raw: &str) -> Option<PaneKind> {
        match raw {
            "terminal" => Some(PaneKind::Terminal),
            "files" => Some(PaneKind::Files),
            "diff" => Some(PaneKind::Diff),
            "commits" => Some(PaneKind::Commits),
            _ => None,
        }
    }
}

// MARK: - Persisted DTOs: PanePayload / TileLayout / LayoutPreset

/// Persisted representation of one tab. Field order below is the Swift
/// source's declaration order (`kind, title, agentSessionId,
/// agentTranscriptPath`); see the module doc comment for why this crate
/// doesn't try to match Swift's (non-deterministic) runtime key order.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PanePayload {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Claude session ID last observed for this terminal tab (tab-scoped
    /// `--resume`).
    #[serde(rename = "agentSessionId", skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    /// Path to the corresponding transcript (JSONL); used to check the
    /// session still exists before resuming.
    #[serde(
        rename = "agentTranscriptPath",
        skip_serializing_if = "Option::is_none"
    )]
    pub agent_transcript_path: Option<String>,
    /// User-assigned tab color (第10波 パーソナライズ): a lowercase
    /// `#rrggbb` string, or `None` for the default. **New-in-Rust key** with
    /// no Swift counterpart -- omitted entirely when `None`
    /// (`skip_serializing_if`), so every layout without a tab color still
    /// serializes byte-identically to what the Swift app's `JSONEncoder`
    /// produced (the golden-fixture contract in `tests/tiling_golden.rs` is
    /// unaffected), and Swift's `JSONDecoder`/older readers simply ignore
    /// the unknown key when it *is* present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Persisted representation of a tile layout (`Codable` in Swift). A leaf
/// with exactly one tab writes the **legacy** shape (`paneKind`/`paneTitle`/
/// `paneAgentSessionId`/`paneAgentTranscriptPath`) for backward compat with
/// data written before tabs existed; a leaf with 2+ tabs writes
/// `panes`/`selectedIndex`. A split node writes
/// `orientation`/`ratio`/`children`. See [`PaneTilingModel::snapshot`] /
/// [`PaneTilingModel::model_from`] for the encode/decode rules, which this
/// type intentionally does not enforce itself (a `TileLayout` value can be
/// constructed in "impossible" shapes -- e.g. both `paneKind` and `panes`
/// set -- exactly as it can in Swift; `panes` wins on decode, matching the
/// Swift source's `if let payloads = layout.panes` check ordering).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TileLayout {
    #[serde(rename = "paneKind", skip_serializing_if = "Option::is_none")]
    pub pane_kind: Option<String>,
    #[serde(rename = "paneTitle", skip_serializing_if = "Option::is_none")]
    pub pane_title: Option<String>,
    /// Legacy (single-tab) leaf's Claude session ID. Unknown/harmless key to
    /// pre-tab readers.
    #[serde(rename = "paneAgentSessionId", skip_serializing_if = "Option::is_none")]
    pub pane_agent_session_id: Option<String>,
    /// Legacy (single-tab) leaf's transcript path.
    #[serde(
        rename = "paneAgentTranscriptPath",
        skip_serializing_if = "Option::is_none"
    )]
    pub pane_agent_transcript_path: Option<String>,
    /// Single-tab leaf's user-assigned tab color (第10波; the legacy-shape
    /// counterpart of [`PanePayload::color`] -- see that field's doc comment
    /// for the compatibility contract: new-in-Rust key, omitted when `None`,
    /// ignored by Swift/older readers).
    #[serde(rename = "paneColor", skip_serializing_if = "Option::is_none")]
    pub pane_color: Option<String>,
    /// Tab group (written only when there are 2+ tabs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panes: Option<Vec<PanePayload>>,
    /// Selected tab index; only meaningful when `panes` is set.
    #[serde(rename = "selectedIndex", skip_serializing_if = "Option::is_none")]
    pub selected_index: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TileLayout>>,
}

impl TileLayout {
    /// A copy with all per-tab Claude session info (ID / transcript path)
    /// removed. Presets are "the shape of a layout" shared across every
    /// session, so this strips any one session's resume info before saving
    /// a layout as a preset. Mirrors `TileLayout.strippingAgentSessions()`.
    /// Tab colors (`pane_color`/`PanePayload::color`, 第10波) are
    /// deliberately **kept**: a color is part of the layout's visual shape
    /// (like titles, which are kept too), not one session's identity.
    pub fn stripping_agent_sessions(&self) -> TileLayout {
        let mut copy = self.clone();
        copy.pane_agent_session_id = None;
        copy.pane_agent_transcript_path = None;
        copy.panes = copy.panes.map(|panes| {
            panes
                .into_iter()
                .map(|mut p| {
                    p.agent_session_id = None;
                    p.agent_transcript_path = None;
                    p
                })
                .collect()
        });
        copy.children = copy.children.map(|children| {
            children
                .iter()
                .map(TileLayout::stripping_agent_sessions)
                .collect()
        });
        copy
    }

    /// Serializes to the same JSON *content* `JSONEncoder().encode(_:)`
    /// produces for this type in the Swift app (`SessionStore.encodeLayout`)
    /// -- same keys, same omissions, same string/float formatting -- modulo
    /// object key order, which Swift's own encoder does not keep stable
    /// across runs either. See the module doc comment.
    pub fn to_json(&self) -> String {
        swift_json::to_string(self).expect("TileLayout serialization is infallible")
    }

    /// Parses JSON produced by [`TileLayout::to_json`] or by the Swift app's
    /// `JSONEncoder().encode(_:)` (`SessionStore.decodeLayout`). Unknown
    /// keys are ignored and missing optional keys default to `None`,
    /// matching `JSONDecoder`'s synthesized `init(from:)`.
    pub fn from_json(json: &str) -> serde_json::Result<TileLayout> {
        serde_json::from_str(json)
    }
}

/// A named layout preset, shared across sessions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutPreset {
    pub name: String,
    pub layout: TileLayout,
}

impl LayoutPreset {
    /// Mirrors the Swift `Identifiable` conformance (`var id: String { name }`).
    pub fn id(&self) -> &str {
        &self.name
    }
}

/// A `serde_json::ser::Formatter` that reproduces the two byte-level
/// `JSONEncoder` quirks documented on the module -- `/` escaping and
/// no-trailing-`.0` float formatting -- without attempting (or needing) to
/// match Swift's non-deterministic key order. Used only by
/// [`TileLayout::to_json`].
mod swift_json {
    use serde::Serialize;
    use serde_json::ser::{CompactFormatter, Formatter, Serializer};
    use std::io;

    #[derive(Default)]
    pub(super) struct SwiftCompatFormatter(CompactFormatter);

    impl Formatter for SwiftCompatFormatter {
        /// `CGFloat`/`Double` via `JSONEncoder` render as the shortest
        /// round-trip decimal with no trailing `.0` for integral values
        /// (`1.0` -> `"1"`; verified empirically against the real encoder).
        /// Rust's `f64::to_string()` already produces byte-identical output
        /// for every value this model ever encodes (`ratio` is always
        /// clamped to `[0.05, 0.95]` on decode, and the `defaultLayout()`
        /// ratios are all "nice" fractions) -- no `ryu`/scientific-notation
        /// handling needed.
        fn write_f64<W>(&mut self, writer: &mut W, value: f64) -> io::Result<()>
        where
            W: ?Sized + io::Write,
        {
            writer.write_all(value.to_string().as_bytes())
        }

        /// `serde_json`'s default formatter never escapes `/` (not required
        /// by the JSON spec); `JSONEncoder` always does. This is the hook
        /// `serde_json` calls with runs of string content that don't need
        /// any of *its own* built-in escapes, so it's the right place to
        /// inject `/` -> `\/`.
        fn write_string_fragment<W>(&mut self, writer: &mut W, fragment: &str) -> io::Result<()>
        where
            W: ?Sized + io::Write,
        {
            let bytes = fragment.as_bytes();
            let mut start = 0;
            for (i, &b) in bytes.iter().enumerate() {
                if b == b'/' {
                    if start < i {
                        writer.write_all(&bytes[start..i])?;
                    }
                    writer.write_all(b"\\/")?;
                    start = i + 1;
                }
            }
            writer.write_all(&bytes[start..])
        }
    }

    pub(super) fn to_string<T: Serialize + ?Sized>(value: &T) -> serde_json::Result<String> {
        let mut buf = Vec::new();
        let mut ser = Serializer::with_formatter(&mut buf, SwiftCompatFormatter::default());
        value.serialize(&mut ser)?;
        Ok(String::from_utf8(buf).expect("serde_json only ever writes valid UTF-8"))
    }
}

// MARK: - Domain model: PaneItem / TileNode

/// One tab: a terminal surface, the changed-files list, a diff, or the
/// commit graph.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneItem {
    pub id: PaneId,
    pub kind: PaneKind,
    pub title: String,
    /// Claude session ID last observed for this terminal tab (from hooks);
    /// persisted alongside the layout for tab-scoped `--resume` on next
    /// launch. Always `None` for non-terminal panes.
    pub agent_session_id: Option<String>,
    /// Path to the corresponding transcript (JSONL); used to check the
    /// session still exists before resuming.
    pub agent_transcript_path: Option<String>,
    /// User-assigned tab color (第10波): a lowercase `#rrggbb` string, or
    /// `None` for the default. Persisted through the layout DTOs
    /// ([`PanePayload::color`] / [`TileLayout::pane_color`]) -- see
    /// `PanePayload::color`'s doc comment for the JSON-compatibility
    /// contract. Like `title`, this is pure display state; nothing in this
    /// crate interprets the string.
    pub color: Option<String>,
}

impl PaneItem {
    pub fn new(kind: PaneKind, title: impl Into<String>) -> Self {
        PaneItem {
            id: PaneId::new(),
            kind,
            title: title.into(),
            agent_session_id: None,
            agent_transcript_path: None,
            color: None,
        }
    }

    pub fn with_agent_session(
        kind: PaneKind,
        title: impl Into<String>,
        agent_session_id: impl Into<String>,
        agent_transcript_path: impl Into<String>,
    ) -> Self {
        PaneItem {
            id: PaneId::new(),
            kind,
            title: title.into(),
            agent_session_id: Some(agent_session_id.into()),
            agent_transcript_path: Some(agent_transcript_path.into()),
            color: None,
        }
    }

    /// Whether this pane should auto-resume its Claude session on restore
    /// (`labolabo-app`'s per-tab-resume-at-spawn-time flow -- see
    /// `crate::hook_settings::claude_resume_command`). A pure gate, taking
    /// the transcript's existence as a caller-supplied bool so the
    /// filesystem check itself stays at the I/O boundary. Mirrors the Swift
    /// `resumable` filter in `ContentView.swift`'s
    /// `triggerAutoResumeIfNeeded`: needs a non-empty `agent_session_id`,
    /// and if an `agent_transcript_path` was recorded, it must actually
    /// exist on disk (an unrecorded path -- old data -- is tried as before,
    /// matching docs/hooks-protocol.md §6's resume guard).
    pub fn is_resumable(&self, transcript_exists: bool) -> bool {
        let Some(id) = &self.agent_session_id else {
            return false;
        };
        if id.is_empty() {
            return false;
        }
        if self.agent_transcript_path.is_some() && !transcript_exists {
            return false;
        }
        true
    }
}

/// Minimum a split node's `ratio` (first child's fraction) may be set to,
/// interactively or otherwise -- via [`TileNode::set_ratio`]/
/// [`PaneTilingModel::set_split_ratio`]. Keeps a dragged divider from
/// collapsing a child to (or past) zero width/height. Mirrors the clamp
/// `task_workspace::render_tile` (the Rust UI layer) already applied only
/// at *render* time (`(node.ratio as f32).clamp(0.05, 0.95)`) before this
/// constant existed -- centralizing it here means a dragged ratio is
/// clamped once, at the source, so the persisted value itself can never
/// drift outside range either (previously only the on-screen split ever
/// respected this bound; a ratio written directly to `TileNode::ratio` by
/// some other caller could not).
pub const MIN_SPLIT_RATIO: f64 = 0.05;
/// Maximum a split node's `ratio` may be set to -- see [`MIN_SPLIT_RATIO`].
pub const MAX_SPLIT_RATIO: f64 = 0.95;

/// A node in the binary tile tree: a leaf (tab group, `panes` non-empty) or
/// a 2-way split (`panes` empty, two `children`).
#[derive(Debug, Clone, PartialEq)]
pub struct TileNode {
    pub id: NodeId,
    /// Leaf's tabs, display order == tab bar order.
    pub panes: Vec<PaneItem>,
    /// Selected tab index. Must always stay in range when `panes` changes --
    /// see [`TileNode::remove_pane`] -- so [`TileNode::selected_pane`] never
    /// has to silently disagree with what's persisted.
    pub selected_index: usize,
    pub orientation: TileOrientation,
    pub children: Vec<TileNode>,
    /// First child's fraction of the split (0…1). Persisted across rebuilds.
    pub ratio: f64,
}

impl Default for TileNode {
    /// An empty (zero-child) split node. Used only as a transient
    /// placeholder inside [`PaneTilingModel::add_pane`]'s ownership shuffle
    /// -- never a shape a caller observes.
    fn default() -> Self {
        TileNode {
            id: NodeId::new(),
            panes: Vec::new(),
            selected_index: 0,
            orientation: TileOrientation::Horizontal,
            children: Vec::new(),
            ratio: 0.5,
        }
    }
}

impl TileNode {
    pub fn leaf(pane: PaneItem) -> Self {
        TileNode {
            id: NodeId::new(),
            panes: vec![pane],
            selected_index: 0,
            orientation: TileOrientation::Horizontal,
            children: Vec::new(),
            ratio: 0.5,
        }
    }

    /// A leaf carrying a whole tab group (used when moving a group's tabs +
    /// selection to a new node).
    pub fn tab_group(panes: Vec<PaneItem>, selected_index: usize) -> Self {
        TileNode {
            id: NodeId::new(),
            panes,
            selected_index,
            orientation: TileOrientation::Horizontal,
            children: Vec::new(),
            ratio: 0.5,
        }
    }

    pub fn split(orientation: TileOrientation, ratio: f64, children: Vec<TileNode>) -> Self {
        TileNode {
            id: NodeId::new(),
            panes: Vec::new(),
            selected_index: 0,
            orientation,
            children,
            ratio,
        }
    }

    pub fn is_leaf(&self) -> bool {
        !self.panes.is_empty()
    }

    /// The tab on display. Falls back to `first` if `selected_index` is
    /// somehow out of range, so exactly one tab is always returned.
    pub fn selected_pane(&self) -> Option<&PaneItem> {
        self.panes
            .get(self.selected_index)
            .or_else(|| self.panes.first())
    }

    // MARK: tree helpers

    pub fn leaves(&self) -> Vec<&TileNode> {
        if self.is_leaf() {
            vec![self]
        } else {
            self.children.iter().flat_map(TileNode::leaves).collect()
        }
    }

    /// The leaf node containing `pane_id`, if any.
    pub fn find_leaf(&self, pane_id: PaneId) -> Option<&TileNode> {
        if self.is_leaf() {
            return if self.panes.iter().any(|p| p.id == pane_id) {
                Some(self)
            } else {
                None
            };
        }
        self.children.iter().find_map(|c| c.find_leaf(pane_id))
    }

    fn find_leaf_mut(&mut self, pane_id: PaneId) -> Option<&mut TileNode> {
        if self.is_leaf() {
            return if self.panes.iter().any(|p| p.id == pane_id) {
                Some(self)
            } else {
                None
            };
        }
        self.children
            .iter_mut()
            .find_map(|c| c.find_leaf_mut(pane_id))
    }

    fn find_pane_mut(&mut self, pane_id: PaneId) -> Option<&mut PaneItem> {
        if self.is_leaf() {
            return self.panes.iter_mut().find(|p| p.id == pane_id);
        }
        self.children
            .iter_mut()
            .find_map(|c| c.find_pane_mut(pane_id))
    }

    /// The parent of the node identified by `child_id`, and that child's
    /// index among its parent's children.
    pub fn find_parent(&self, child_id: NodeId) -> Option<(&TileNode, usize)> {
        if let Some(idx) = self.children.iter().position(|c| c.id == child_id) {
            return Some((self, idx));
        }
        self.children.iter().find_map(|c| c.find_parent(child_id))
    }

    fn find_parent_mut(&mut self, child_id: NodeId) -> Option<(&mut TileNode, usize)> {
        if let Some(idx) = self.children.iter().position(|c| c.id == child_id) {
            return Some((self, idx));
        }
        self.children
            .iter_mut()
            .find_map(|c| c.find_parent_mut(child_id))
    }

    /// The node (leaf or split) whose own `id` is `node_id`, anywhere in
    /// this subtree, mutably -- `None` if not found. Unlike
    /// [`Self::find_leaf_mut`]/[`Self::find_parent_mut`] (which look a node
    /// up by a *leaf's* [`PaneId`] or by a *child's* identity respectively),
    /// this matches a node's own [`NodeId`] directly -- what an interactive
    /// divider-drag handler has on hand (a split node's own `id`, read once
    /// from the tree when the drag starts). Used by
    /// [`PaneTilingModel::set_split_ratio`].
    pub fn find_node_mut(&mut self, node_id: NodeId) -> Option<&mut TileNode> {
        if self.id == node_id {
            return Some(self);
        }
        self.children
            .iter_mut()
            .find_map(|c| c.find_node_mut(node_id))
    }

    /// Sets this node's `ratio`, clamped to between [`MIN_SPLIT_RATIO`] and
    /// [`MAX_SPLIT_RATIO`]. A non-finite
    /// (`NaN`/`inf`) input -- reachable from a degenerate divide-by-zero
    /// pixel computation mid-drag, e.g. a momentarily zero-width/height
    /// split container during a concurrent window resize -- is ignored
    /// outright, leaving the existing ratio untouched, rather than
    /// corrupting it. Returns the ratio actually stored (the clamped new
    /// value, or the unchanged old one for a rejected non-finite input), so
    /// a caller driving a live divider drag can paint the split at exactly
    /// what was applied even where it differs from what was asked for
    /// (clamped at an edge).
    ///
    /// Meaningful only on a split node -- a leaf's `ratio` field is unused
    /// (always its construction-time default, `0.5`) and irrelevant to
    /// rendering, but this method doesn't distinguish (mirrors every other
    /// direct-field-write this struct already exposes: `ratio` itself is
    /// `pub`).
    pub fn set_ratio(&mut self, ratio: f64) -> f64 {
        if ratio.is_finite() {
            self.ratio = ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
        }
        self.ratio
    }

    // MARK: mutations (operate on `self` as the node being changed)

    /// Removes tab `index` from this leaf, returning it, and adjusts
    /// `selected_index` so "the tab you were looking at doesn't jump": if
    /// the removed tab was before the selection, the selection shifts down
    /// by one (later tabs slid into its place); the result is always
    /// clamped into range (or reset to `0` once empty). Mirrors
    /// `PaneTilingModel.removePane(at:from:)`.
    fn remove_pane(&mut self, index: usize) -> PaneItem {
        let removed = self.panes.remove(index);
        if index < self.selected_index {
            self.selected_index -= 1;
        }
        self.selected_index = if self.panes.is_empty() {
            0
        } else {
            self.selected_index.min(self.panes.len() - 1)
        };
        removed
    }

    /// Turns this leaf (which keeps its whole current tab group) into a
    /// 2-way split: the existing tabs stay together as one child, and
    /// `moved_pane` becomes a lone new leaf on the `edge` side. Mirrors
    /// `PaneTilingModel.splitLeafOut`.
    fn split_leaf_out(&mut self, moved_pane: PaneItem, edge: DropEdge) {
        let orientation = if matches!(edge, DropEdge::Left | DropEdge::Right) {
            TileOrientation::Horizontal
        } else {
            TileOrientation::Vertical
        };
        let moved_on_second = matches!(edge, DropEdge::Right | DropEdge::Bottom);
        let keep_selected_index = self.selected_index;
        let keep_panes = std::mem::take(&mut self.panes);
        let keep = TileNode::tab_group(keep_panes, keep_selected_index);
        let moved = TileNode::leaf(moved_pane);
        self.selected_index = 0;
        self.orientation = orientation;
        self.children = if moved_on_second {
            vec![keep, moved]
        } else {
            vec![moved, keep]
        };
        self.ratio = 0.5;
    }

    /// Collapses this (split) node into the content of
    /// `self.children[sibling_index]`, discarding both current children (the
    /// other child -- the one being closed/detached -- and the sibling node
    /// itself, whose fields live on merged directly into `self`). Mirrors
    /// `PaneTilingModel.collapse(_:into:)`, with the
    /// `parent.children[index == 0 ? 1 : 0]` sibling lookup folded in.
    fn collapse_with_sibling(&mut self, sibling_index: usize) {
        let sibling = self.children.remove(sibling_index);
        self.panes = sibling.panes;
        self.selected_index = sibling.selected_index;
        self.orientation = sibling.orientation;
        self.children = sibling.children;
        self.ratio = sibling.ratio;
    }
}

/// Where a dragged pane is dropped relative to the target pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropEdge {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

/// Resolves a drop point within a `width` x `height` rectangle into a
/// [`DropEdge`] zone: a drop in the outer 25% margin on a side is that
/// side's edge (a split), and the inner 50% x 50% is [`DropEdge::Center`]
/// (a tab merge). `x`/`y` are relative to the rectangle's own top-left
/// origin.
///
/// This is `plans/012-task-model-and-control-cli.md` §3's ported-behavior
/// geometry: `app/Sources/PaneTiling.swift`'s `PaneFrameView.edge(at:)` used
/// the same 25%/75% thresholds, but against AppKit's default *flipped*
/// view coordinates (Y increases upward, so "top" was the *high*-Y branch).
/// gpui (like most compositors -- and this function) uses top-left-origin,
/// Y-increases-downward coordinates instead, so "top" here is the low-Y
/// branch. The zone *names* and thresholds are what's ported; the raw axis
/// direction deliberately isn't (there's nothing to port there -- it's just
/// each platform's own coordinate convention), matching this wave's brief
/// that pixel-for-pixel AppKit fidelity isn't the goal.
///
/// Degenerate rectangles (`width <= 0.0` or `height <= 0.0`) resolve to
/// [`DropEdge::Center`] -- there's no meaningful edge to compute a fraction
/// against, and a whole-pane tab-merge is the least surprising fallback.
pub fn drop_edge_for_point(width: f32, height: f32, x: f32, y: f32) -> DropEdge {
    if width <= 0.0 || height <= 0.0 {
        return DropEdge::Center;
    }
    let rx = x / width;
    let ry = y / height;
    if rx < 0.25 {
        return DropEdge::Left;
    }
    if rx > 0.75 {
        return DropEdge::Right;
    }
    if ry < 0.25 {
        return DropEdge::Top;
    }
    if ry > 0.75 {
        return DropEdge::Bottom;
    }
    DropEdge::Center
}

/// Operations the model asks the UI (coordinator) to perform: sending text
/// to a terminal, and requesting that a pane receive key focus after the
/// next rebuild. Mirrors the Swift `PaneTilingActions` protocol (kept
/// UI-independent the same way: only terminal text and focus-pane
/// bookkeeping, nothing AppKit-specific).
pub trait PaneTilingActions {
    /// The terminal pane that should receive key focus after the next
    /// reconcile.
    fn pending_focus_pane_id(&self) -> Option<PaneId>;
    fn set_pending_focus_pane_id(&mut self, pane_id: Option<PaneId>);
    /// Sends `text` (a command) to the given pane's terminal.
    fn schedule_send(&mut self, pane_id: PaneId, text: String);
}

/// One session's tile tree, plus the bookkeeping (`revision`,
/// `on_layout_changed`) the UI/persistence layer needs.
pub struct PaneTilingModel {
    pub root: TileNode,
    revision: u64,
    /// The UI (AppKit coordinator, or its future Rust-UI equivalent)'s
    /// operations surface: terminal text send + focus handoff. Swift holds
    /// this as `weak var coordinator` to avoid a retain cycle with the
    /// AppKit view controller that owns the model; that concern doesn't
    /// apply to an owned `Box` here (there is no analogous cycle in a
    /// pure-logic core with no owning UI object), so this port drops the
    /// weak-reference detail while keeping the trait-based operations
    /// surface itself.
    pub coordinator: Option<Box<dyn PaneTilingActions>>,
    /// Called on every structural mutation (add/close/split/move/select-tab)
    /// and on `record_agent_session`, for the caller to persist the layout.
    /// Ratio changes (drag-resize) are *not* routed through this model at
    /// all in the Swift source -- the UI reads/writes `TileNode.ratio`
    /// directly and snapshots+saves on drag end -- so there's nothing to
    /// call here for that case either.
    pub on_layout_changed: Option<Box<dyn FnMut()>>,
}

impl PaneTilingModel {
    pub fn new(root: TileNode) -> Self {
        PaneTilingModel {
            root,
            revision: 0,
            coordinator: None,
            on_layout_changed: None,
        }
    }

    /// Bumped on every structural mutation, so a UI layer knows to
    /// re-reconcile its view tree. `select_tab`/`record_agent_session`
    /// intentionally do *not* bump this (see their doc comments) --
    /// mirrors the Swift `revision` property exactly.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        if let Some(cb) = self.on_layout_changed.as_mut() {
            cb();
        }
    }

    /// Terminal on top; bottom row = commit graph | changed-files | diff
    /// (1:1:2).
    pub fn default_layout() -> PaneTilingModel {
        let terminal = TileNode::leaf(PaneItem::new(
            PaneKind::Terminal,
            PaneKind::Terminal.default_title(),
        ));
        let commits = TileNode::leaf(PaneItem::new(
            PaneKind::Commits,
            PaneKind::Commits.default_title(),
        ));
        let files = TileNode::leaf(PaneItem::new(
            PaneKind::Files,
            PaneKind::Files.default_title(),
        ));
        let diff = TileNode::leaf(PaneItem::new(
            PaneKind::Diff,
            PaneKind::Diff.default_title(),
        ));
        // files : diff = 1 : 2 -> files takes 1/3 of (files+diff)
        let files_and_diff =
            TileNode::split(TileOrientation::Horizontal, 1.0 / 3.0, vec![files, diff]);
        // commits : (files+diff) = 1 : 3 -> commits takes 1/4 of the bottom row
        let bottom = TileNode::split(
            TileOrientation::Horizontal,
            0.25,
            vec![commits, files_and_diff],
        );
        let root = TileNode::split(TileOrientation::Vertical, 0.55, vec![terminal, bottom]);
        PaneTilingModel::new(root)
    }

    /// New terminal pane, then (once the shell has launched) sends
    /// `command` to it -- used to launch Claude etc. Requests key focus for
    /// the new pane.
    pub fn launch_in_new_terminal(&mut self, title: impl Into<String>, command: impl Into<String>) {
        let pane = PaneItem::new(PaneKind::Terminal, title.into());
        let pane_id = pane.id;
        self.add_pane(pane);
        if let Some(coordinator) = self.coordinator.as_mut() {
            coordinator.set_pending_focus_pane_id(Some(pane_id));
            coordinator.schedule_send(pane_id, command.into());
        }
    }

    /// Sends `command` to the first existing terminal pane (tree order), if
    /// any -- used for auto-resume, so it doesn't add a new pane, just types
    /// the resume command into the just-restored shell.
    pub fn send_to_existing_terminal(&mut self, command: impl Into<String>) -> bool {
        let Some(pane_id) = self
            .panes()
            .into_iter()
            .find(|p| p.kind == PaneKind::Terminal)
            .map(|p| p.id)
        else {
            return false;
        };
        if let Some(coordinator) = self.coordinator.as_mut() {
            coordinator.schedule_send(pane_id, command.into());
        }
        true
    }

    /// Terminal tabs across every leaf, tree order -- used to walk tabs for
    /// per-tab resume.
    pub fn terminal_panes(&self) -> Vec<&PaneItem> {
        self.panes()
            .into_iter()
            .filter(|p| p.kind == PaneKind::Terminal)
            .collect()
    }

    /// Sends `command` to a specific pane's terminal (per-tab resume).
    pub fn send_to_terminal(&mut self, pane_id: PaneId, command: impl Into<String>) {
        if let Some(coordinator) = self.coordinator.as_mut() {
            coordinator.schedule_send(pane_id, command.into());
        }
    }

    /// Records a (pane, Claude session ID, transcript path) association
    /// from hooks, persisted alongside the layout. Doesn't change tree
    /// structure, so this doesn't `bump()` (no reconcile needed) -- it only
    /// fires `on_layout_changed` (matches the Swift source exactly, and see
    /// [`PaneTilingModel::record_agent_session`]'s module-doc note on why
    /// this takes a [`PaneId`] directly rather than a `UUID` string).
    pub fn record_agent_session(
        &mut self,
        id: impl Into<String>,
        pane_id: PaneId,
        transcript_path: Option<String>,
    ) {
        let id = id.into();
        let Some(pane) = self.root.find_pane_mut(pane_id) else {
            return;
        };
        if pane.kind != PaneKind::Terminal {
            return;
        }
        let new_transcript = transcript_path.or_else(|| pane.agent_transcript_path.clone());
        if pane.agent_session_id.as_deref() == Some(id.as_str())
            && pane.agent_transcript_path == new_transcript
        {
            return;
        }
        pane.agent_session_id = Some(id);
        pane.agent_transcript_path = new_transcript;
        if let Some(cb) = self.on_layout_changed.as_mut() {
            cb();
        }
    }

    /// All tabs across every leaf (tree DFS order), including hidden ones --
    /// deliberately, so a reconcile pass sees every pane's ID as "still
    /// live" and doesn't tear down a hidden tab's pty.
    pub fn panes(&self) -> Vec<&PaneItem> {
        self.root
            .leaves()
            .into_iter()
            .flat_map(|n| n.panes.iter())
            .collect()
    }

    pub fn has_pane(&self, kind: PaneKind) -> bool {
        self.panes().iter().any(|p| p.kind == kind)
    }

    /// The `PaneKind` of each leaf's *currently selected* tab -- i.e. what's
    /// actually front-facing on screen right now, tree order. Unlike
    /// [`Self::panes`] (every tab of every leaf, including hidden
    /// non-selected ones), a leaf with 2+ tabs contributes exactly one entry
    /// here: its `selected_pane()`'s kind. Used by the UI layer (`labolabo-
    /// app`'s `LaboLaboApp::git_pane_state_needed`) to decide whether *any*
    /// Git-kind display (`Files`/`Diff`/`Commits`) is currently visible for
    /// this Task, so its Git state's background refresh/watch can be gated
    /// on "is anything actually showing it" rather than only the fixed
    /// side-pane's own visibility flag.
    pub fn visible_pane_kinds(&self) -> Vec<PaneKind> {
        self.root
            .leaves()
            .into_iter()
            .filter_map(|leaf| leaf.selected_pane().map(|p| p.kind))
            .collect()
    }

    // MARK: - layout serialization / restore / presets

    /// Builds a model from a persisted layout (`None` if invalid).
    pub fn model_from(layout: &TileLayout) -> Option<PaneTilingModel> {
        let node = decode(layout)?;
        Some(PaneTilingModel::new(node))
    }

    /// The current tile tree as a persisted snapshot.
    pub fn snapshot(&self) -> TileLayout {
        encode(&self.root)
    }

    /// Replaces the tree wholesale with a persisted layout (preset or
    /// per-session). No-op if `layout` doesn't decode.
    pub fn apply(&mut self, layout: &TileLayout) {
        let Some(node) = decode(layout) else {
            return;
        };
        self.root = node;
        self.bump();
    }

    pub fn reset_to_default(&mut self) {
        self.root = PaneTilingModel::default_layout().root;
        self.bump();
    }

    // MARK: - mutations

    pub fn split(&mut self, pane_id: PaneId, orientation: TileOrientation, new_pane: PaneItem) {
        let Some(node) = self.root.find_leaf_mut(pane_id) else {
            return;
        };
        let edge = if orientation == TileOrientation::Horizontal {
            DropEdge::Right
        } else {
            DropEdge::Bottom
        };
        node.split_leaf_out(new_pane, edge);
        self.bump();
    }

    /// Adds `new_pane` as a new tab in the tab group (leaf) that contains
    /// `pane_id`, selecting it. Returns whether `pane_id` was found (a
    /// caller that already spawned a resource for `new_pane`, e.g. a PTY,
    /// needs to know whether to tear it back down on failure). No-op (and
    /// returns `false`) if `pane_id` isn't found.
    ///
    /// Not present in the Swift source -- `PaneTilingModel.swift`'s UI never
    /// needed a keyboard-driven "new tab in this pane" affordance (tabs are
    /// only ever created by dragging one leaf's tab onto another's center,
    /// i.e. [`PaneTilingModel::move_pane`]'s [`DropEdge::Center`] case, which
    /// requires the pane to already exist in the tree). This is the same
    /// "append and select" logic as that case, but for a pane that doesn't
    /// exist in the tree yet -- e.g. a Rust-UI-only "new tab" shortcut
    /// (Cmd+T) with no drag source to move.
    pub fn add_tab(&mut self, pane_id: PaneId, new_pane: PaneItem) -> bool {
        let Some(node) = self.root.find_leaf_mut(pane_id) else {
            return false;
        };
        node.panes.push(new_pane);
        node.selected_index = node.panes.len() - 1;
        self.bump();
        true
    }

    pub fn add_pane(&mut self, pane: PaneItem) {
        let added = TileNode::leaf(pane);
        let old_root = std::mem::take(&mut self.root);
        self.root = TileNode::split(TileOrientation::Horizontal, 0.7, vec![old_root, added]);
        self.bump();
    }

    pub fn add_pane_if_absent(&mut self, kind: PaneKind, title: impl Into<String>) {
        if self.has_pane(kind) {
            return;
        }
        self.add_pane(PaneItem::new(kind, title.into()));
    }

    /// Closes a tab/pane. Returns the ID of the tab that becomes front-facing
    /// as a result (for focus handoff), or `None` if nothing was closed or
    /// no front-facing tab is well-defined. Callers may ignore the result
    /// (mirrors Swift's `@discardableResult`).
    pub fn close(&mut self, pane_id: PaneId) -> Option<PaneId> {
        let (node_id, index, pane_count) = {
            let node = self.root.find_leaf(pane_id)?;
            let index = node.panes.iter().position(|p| p.id == pane_id)?;
            (node.id, index, node.panes.len())
        };

        if pane_count > 1 {
            // Group has multiple tabs -> close just this one (siblings' ptys
            // are preserved).
            let node = self
                .root
                .find_leaf_mut(pane_id)
                .expect("leaf located just above");
            node.remove_pane(index);
            let revealed = node.selected_pane().map(|p| p.id);
            self.bump();
            return revealed;
        }

        // Last tab in the group: collapse the parent into the sibling, as
        // before. A lone root leaf has no parent, so it's kept as a no-op
        // (at least one pane always remains).
        let (parent, p_index) = self.root.find_parent_mut(node_id)?;
        let sibling_index = if p_index == 0 { 1 } else { 0 };
        parent.collapse_with_sibling(sibling_index);
        // If the collapsed parent is now itself a leaf (tab group), its
        // selected tab becomes front-facing.
        let revealed = parent.selected_pane().map(|p| p.id);
        self.bump();
        revealed
    }

    /// Moves a tab/pane. Returns `true` if the tree actually changed (used
    /// to decide whether to move focus). Callers may ignore the result
    /// (mirrors Swift's `@discardableResult`).
    pub fn move_pane(&mut self, source_id: PaneId, target_id: PaneId, edge: DropEdge) -> bool {
        let Some((source_leaf_id, source_index, target_leaf_id)) = (|| {
            let source_leaf = self.root.find_leaf(source_id)?;
            let source_index = source_leaf.panes.iter().position(|p| p.id == source_id)?;
            let target_leaf = self.root.find_leaf(target_id)?;
            Some((source_leaf.id, source_index, target_leaf.id))
        })() else {
            return false;
        };

        // Dropped within the same group.
        if source_leaf_id == target_leaf_id {
            if edge == DropEdge::Center {
                return false; // already merged (center of its own group) = no-op
            }
            let node = self
                .root
                .find_leaf_mut(source_id)
                .expect("leaf located just above");
            if node.panes.len() <= 1 {
                return false; // dropping a lone tab on its own edge is meaningless
            }
            let source_pane = node.remove_pane(source_index);
            // Split the remaining group toward `edge`, with `source` becoming
            // an independent new leaf on the `edge` side.
            node.split_leaf_out(source_pane, edge);
            self.bump();
            return true;
        }

        // Dropped on another group: remove `source` from its original group
        // (collapsing the parent away if that empties it).
        let source_leaf = self
            .root
            .find_leaf_mut(source_id)
            .expect("leaf located just above");
        let source_pane = source_leaf.remove_pane(source_index);
        let source_leaf_emptied = source_leaf.panes.is_empty();
        if source_leaf_emptied {
            self.detach(source_leaf_id);
        }

        // The tree changed, so re-locate the target (it may have moved or,
        // in a two-leaf tree, disappeared entirely if it was the sibling
        // that got collapsed upward -- same as `move`'s own bump-and-return).
        let Some(target) = self.root.find_leaf_mut(target_id) else {
            self.bump();
            return true;
        };
        if edge == DropEdge::Center {
            // Tab merge: append and select.
            target.panes.push(source_pane);
            target.selected_index = target.panes.len() - 1;
        } else {
            target.split_leaf_out(source_pane, edge);
        }
        self.bump();
        true
    }

    /// Removes a leaf from the tree by collapsing its parent into the
    /// sibling (no `bump()` -- the caller bumps once for the whole
    /// operation). Mirrors `PaneTilingModel.detach`.
    fn detach(&mut self, node_id: NodeId) {
        if let Some((parent, index)) = self.root.find_parent_mut(node_id) {
            let sibling_index = if index == 0 { 1 } else { 0 };
            parent.collapse_with_sibling(sibling_index);
        }
    }

    /// Changes which tab is selected. Doesn't change tree structure, so
    /// this doesn't `bump()` (no reconcile needed) -- but does fire
    /// `on_layout_changed` so the selection gets persisted. The `isHidden`
    /// view swap itself is a UI-layer concern, out of scope here.
    pub fn select_tab(&mut self, pane_id: PaneId) {
        let Some(node) = self.root.find_leaf_mut(pane_id) else {
            return;
        };
        let Some(i) = node.panes.iter().position(|p| p.id == pane_id) else {
            return;
        };
        if node.selected_index == i {
            return;
        }
        node.selected_index = i;
        if let Some(cb) = self.on_layout_changed.as_mut() {
            cb();
        }
    }

    /// Direct mutable access to a pane anywhere in the tree by ID. Not
    /// present as a named method in the Swift source (it mutates
    /// `PaneItem`s in place through whatever array reference it already
    /// has, since they're class instances there); exposed here because this
    /// port's owned-tree design needs an explicit accessor for the same
    /// thing (used by tests and available for UI-layer code that needs to
    /// mutate a pane's title etc.).
    pub fn pane_mut(&mut self, pane_id: PaneId) -> Option<&mut PaneItem> {
        self.root.find_pane_mut(pane_id)
    }

    /// Updates a split node's `ratio` (e.g. from an interactive divider
    /// drag), clamped via [`TileNode::set_ratio`]. Returns `false` (a
    /// silent no-op) if `node_id` doesn't resolve to any node in this tree
    /// -- e.g. the tree was restructured (a pane closed/moved) while a drag
    /// was in flight; the caller simply stops applying further updates for
    /// that now-stale drag rather than panicking or resurrecting a node.
    ///
    /// Deliberately does **not** `bump()`/fire `on_layout_changed`, mirroring
    /// the Swift source's own documented design (see this struct's
    /// `on_layout_changed` field doc comment: "Ratio changes (drag-resize)
    /// are *not* routed through this model at all... the UI reads/writes
    /// `TileNode.ratio` directly and snapshots+saves on drag end"). A ratio
    /// change touches no tree structure, so there is nothing for a UI
    /// reconcile pass to redo; firing a persistence callback on every one
    /// of a drag's many per-frame ratio updates would also be wasteful --
    /// the caller is expected to explicitly snapshot+persist once itself
    /// when the drag ends (e.g. on mouse-up), not on every intermediate
    /// ratio, exactly as the UI layer already does for a tab-drag-and-drop
    /// move (see `finish_pane_drag_drop` in the Rust UI layer).
    pub fn set_split_ratio(&mut self, node_id: NodeId, ratio: f64) -> bool {
        match self.root.find_node_mut(node_id) {
            Some(node) => {
                node.set_ratio(ratio);
                true
            }
            None => false,
        }
    }
}

// MARK: - encode / decode (TileNode <-> TileLayout)

fn encode(node: &TileNode) -> TileLayout {
    if node.is_leaf() {
        // Backward compat: a single tab writes the legacy shape
        // (paneKind/paneTitle) so pre-tab readers can still load it; only
        // 2+ tabs use the new panes/selectedIndex shape.
        if node.panes.len() == 1 {
            let pane = &node.panes[0];
            return TileLayout {
                pane_kind: Some(pane.kind.raw_value().to_string()),
                pane_title: Some(pane.title.clone()),
                pane_agent_session_id: pane.agent_session_id.clone(),
                pane_agent_transcript_path: pane.agent_transcript_path.clone(),
                pane_color: pane.color.clone(),
                ..Default::default()
            };
        }
        return TileLayout {
            panes: Some(
                node.panes
                    .iter()
                    .map(|p| PanePayload {
                        kind: p.kind.raw_value().to_string(),
                        title: Some(p.title.clone()),
                        agent_session_id: p.agent_session_id.clone(),
                        agent_transcript_path: p.agent_transcript_path.clone(),
                        color: p.color.clone(),
                    })
                    .collect(),
            ),
            selected_index: Some(node.selected_index as i64),
            ..Default::default()
        };
    }
    TileLayout {
        orientation: Some(
            if node.orientation == TileOrientation::Vertical {
                "vertical"
            } else {
                "horizontal"
            }
            .to_string(),
        ),
        ratio: Some(node.ratio),
        children: Some(node.children.iter().map(encode).collect()),
        ..Default::default()
    }
}

fn decode(layout: &TileLayout) -> Option<TileNode> {
    // New-shape tab group takes priority. Entries with an unrecognized
    // `kind` are dropped; if *all* of them are invalid, the leaf doesn't
    // exist and this returns `None`.
    if let Some(payloads) = &layout.panes {
        let items: Vec<PaneItem> = payloads
            .iter()
            .filter_map(|payload| {
                let kind = PaneKind::from_raw_value(&payload.kind)?;
                Some(PaneItem {
                    id: PaneId::new(),
                    kind,
                    title: payload
                        .title
                        .clone()
                        .unwrap_or_else(|| kind.default_title().to_string()),
                    agent_session_id: payload.agent_session_id.clone(),
                    agent_transcript_path: payload.agent_transcript_path.clone(),
                    color: payload.color.clone(),
                })
            })
            .collect();
        if items.is_empty() {
            return None;
        }
        let idx = (layout.selected_index.unwrap_or(0).max(0) as usize).min(items.len() - 1);
        return Some(TileNode::tab_group(items, idx));
    }
    // Legacy shape: single leaf.
    if let Some(kind_raw) = &layout.pane_kind {
        let kind = PaneKind::from_raw_value(kind_raw)?;
        return Some(TileNode::leaf(PaneItem {
            id: PaneId::new(),
            kind,
            title: layout
                .pane_title
                .clone()
                .unwrap_or_else(|| kind.default_title().to_string()),
            agent_session_id: layout.pane_agent_session_id.clone(),
            agent_transcript_path: layout.pane_agent_transcript_path.clone(),
            color: layout.pane_color.clone(),
        }));
    }
    let children = layout.children.as_ref()?;
    if children.len() < 2 {
        return None;
    }
    // Any single child failing to decode fails the whole split (unlike the
    // per-tab drop-invalid-entries behavior above).
    let nodes: Vec<TileNode> = children.iter().map(decode).collect::<Option<_>>()?;
    if nodes.len() != children.len() {
        return None;
    }
    let orientation = if layout.orientation.as_deref() == Some("vertical") {
        TileOrientation::Vertical
    } else {
        TileOrientation::Horizontal
    };
    let ratio = layout.ratio.unwrap_or(0.5).clamp(0.05, 0.95);
    Some(TileNode::split(orientation, ratio, nodes))
}

// MARK: - tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn approx_eq(a: f64, b: f64) {
        assert!((a - b).abs() < 0.0001, "{a} !~= {b}");
    }

    /// A single-leaf model built directly from the real constructors, like
    /// the Swift test file's `makeSinglePaneModel`.
    fn make_single_pane_model(kind: PaneKind) -> PaneTilingModel {
        PaneTilingModel::new(TileNode::leaf(PaneItem::new(kind, kind.default_title())))
    }

    // MARK: - PaneKind::default_title

    /// Port of `testDefaultTitlePerKind`.
    #[test]
    fn default_title_per_kind() {
        assert_eq!(PaneKind::Terminal.default_title(), "端末");
        assert_eq!(PaneKind::Files.default_title(), "変更ファイル");
        assert_eq!(PaneKind::Commits.default_title(), "履歴");
        assert_eq!(PaneKind::Diff.default_title(), "Diff");
    }

    /// Port of `testDefaultTitlesAreDistinctAndNonEmpty`.
    #[test]
    fn default_titles_are_distinct_and_non_empty() {
        let titles = [
            PaneKind::Terminal.default_title(),
            PaneKind::Files.default_title(),
            PaneKind::Diff.default_title(),
            PaneKind::Commits.default_title(),
        ];
        let unique: std::collections::HashSet<_> = titles.iter().collect();
        assert_eq!(unique.len(), 4, "default titles should all be distinct");
        assert!(titles.iter().all(|t| !t.is_empty()));
    }

    /// Port of `testPaneKindRawValuesAreStable`.
    #[test]
    fn pane_kind_raw_values_are_stable() {
        assert_eq!(PaneKind::Terminal.raw_value(), "terminal");
        assert_eq!(PaneKind::Files.raw_value(), "files");
        assert_eq!(PaneKind::Diff.raw_value(), "diff");
        assert_eq!(PaneKind::Commits.raw_value(), "commits");
        assert_eq!(PaneKind::from_raw_value("commits"), Some(PaneKind::Commits));
        assert_eq!(PaneKind::from_raw_value("unknown"), None);
    }

    // MARK: - defaultLayout

    /// Port of `testDefaultLayoutContainsExpectedPaneKinds`.
    #[test]
    fn default_layout_contains_expected_pane_kinds() {
        let model = PaneTilingModel::default_layout();
        assert_eq!(model.panes().len(), 4);
        assert_eq!(
            model.panes().iter().map(|p| p.kind).collect::<Vec<_>>(),
            vec![
                PaneKind::Terminal,
                PaneKind::Commits,
                PaneKind::Files,
                PaneKind::Diff
            ]
        );
        for kind in [
            PaneKind::Terminal,
            PaneKind::Commits,
            PaneKind::Files,
            PaneKind::Diff,
        ] {
            assert!(
                model.has_pane(kind),
                "{kind:?} should be in the default layout"
            );
        }
    }

    /// Port of `testDefaultLayoutRootStructure`.
    #[test]
    fn default_layout_root_structure() {
        let model = PaneTilingModel::default_layout();
        assert!(!model.root.is_leaf());
        assert_eq!(model.root.orientation, TileOrientation::Vertical);
        approx_eq(model.root.ratio, 0.55);
        assert_eq!(model.root.children.len(), 2);
        assert!(model.root.children[0].is_leaf());
        assert_eq!(
            model.root.children[0].selected_pane().map(|p| p.kind),
            Some(PaneKind::Terminal)
        );
    }

    // MARK: - visible_pane_kinds

    #[test]
    fn visible_pane_kinds_lists_every_leafs_selected_kind() {
        let model = PaneTilingModel::default_layout();
        let mut kinds = model.visible_pane_kinds();
        kinds.sort_by_key(|k| k.raw_value());
        let mut expected = vec![
            PaneKind::Terminal,
            PaneKind::Commits,
            PaneKind::Files,
            PaneKind::Diff,
        ];
        expected.sort_by_key(|k| k.raw_value());
        assert_eq!(kinds, expected);
    }

    #[test]
    fn visible_pane_kinds_excludes_a_tab_groups_non_selected_tab() {
        let node = TileNode::tab_group(
            vec![
                PaneItem::new(PaneKind::Terminal, "t"),
                PaneItem::new(PaneKind::Files, "f"),
            ],
            0, // Terminal tab selected -> Files tab is hidden
        );
        let model = PaneTilingModel::new(node);
        assert_eq!(model.visible_pane_kinds(), vec![PaneKind::Terminal]);
    }

    #[test]
    fn visible_pane_kinds_of_single_leaf_is_that_kind() {
        let model = make_single_pane_model(PaneKind::Commits);
        assert_eq!(model.visible_pane_kinds(), vec![PaneKind::Commits]);
    }

    // MARK: - split / add / close

    /// Port of `testSplitIncreasesPaneCount`.
    #[test]
    fn split_increases_pane_count() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        assert_eq!(model.panes().len(), 1);
        let leaf_id = model.panes()[0].id;

        model.split(
            leaf_id,
            TileOrientation::Horizontal,
            PaneItem::new(PaneKind::Files, "f"),
        );

        assert_eq!(model.panes().len(), 2);
        assert!(!model.root.is_leaf());
        assert_eq!(model.root.orientation, TileOrientation::Horizontal);
        assert_eq!(model.root.children.len(), 2);
        assert!(model.has_pane(PaneKind::Files));
        assert!(model.has_pane(PaneKind::Terminal));
    }

    /// Port of `testSplitUnknownPaneIsNoOp`.
    #[test]
    fn split_unknown_pane_is_no_op() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        model.split(
            PaneId::new(),
            TileOrientation::Vertical,
            PaneItem::new(PaneKind::Files, "f"),
        );
        assert_eq!(model.panes().len(), 1);
        assert!(model.root.is_leaf());
    }

    // MARK: - addTab (Rust-only: no Swift port, see the method's doc comment)

    #[test]
    fn add_tab_appends_and_selects_new_tab_in_existing_group() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        model.add_pane(PaneItem::new(PaneKind::Files, "f")); // now a 2-leaf split
        let files_id = model.panes()[1].id;
        assert_eq!(model.panes()[1].kind, PaneKind::Files);

        let ok = model.add_tab(files_id, PaneItem::new(PaneKind::Terminal, "t2"));

        assert!(ok);
        let group = model.root.find_leaf(files_id).unwrap();
        assert_eq!(
            group.panes.iter().map(|p| p.kind).collect::<Vec<_>>(),
            vec![PaneKind::Files, PaneKind::Terminal],
            "the new tab should be appended to files' tab group, not create a new split"
        );
        assert_eq!(
            group.selected_pane().map(|p| p.kind),
            Some(PaneKind::Terminal),
            "the new tab should become selected"
        );
        assert_eq!(model.panes().len(), 3);
    }

    #[test]
    fn add_tab_unknown_pane_is_no_op_and_reports_failure() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        let revision_before = model.revision();

        let ok = model.add_tab(PaneId::new(), PaneItem::new(PaneKind::Files, "f"));

        assert!(!ok);
        assert_eq!(model.panes().len(), 1);
        assert_eq!(
            model.revision(),
            revision_before,
            "a no-op add_tab must not bump revision"
        );
    }

    #[test]
    fn add_tab_bumps_revision_and_fires_layout_callback() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        let only_id = model.panes()[0].id;
        let call_count = Rc::new(RefCell::new(0));
        let call_count_cb = Rc::clone(&call_count);
        model.on_layout_changed = Some(Box::new(move || {
            *call_count_cb.borrow_mut() += 1;
        }));

        model.add_tab(only_id, PaneItem::new(PaneKind::Terminal, "t2"));

        assert_eq!(model.revision(), 1);
        assert_eq!(*call_count.borrow(), 1);
    }

    /// Port of `testAddPaneIncreasesPaneCount`.
    #[test]
    fn add_pane_increases_pane_count() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        model.add_pane(PaneItem::new(PaneKind::Diff, "Diff"));
        assert_eq!(model.panes().len(), 2);
        assert!(model.has_pane(PaneKind::Diff));
    }

    /// Port of `testCloseDecreasesPaneCountAndKeepsSibling`.
    #[test]
    fn close_decreases_pane_count_and_keeps_sibling() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        model.add_pane(PaneItem::new(PaneKind::Files, "f"));
        assert_eq!(model.panes().len(), 2);
        let terminal_id = model.panes()[0].id;
        assert_eq!(model.panes()[0].kind, PaneKind::Terminal);

        model.close(terminal_id);

        assert_eq!(model.panes().len(), 1);
        assert_eq!(
            model.panes()[0].kind,
            PaneKind::Files,
            "closed pane's sibling (files) should remain"
        );
    }

    /// Port of `testCloseRootOnlyPaneIsNoOp`.
    #[test]
    fn close_root_only_pane_is_no_op() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        let only_id = model.panes()[0].id;
        let revealed = model.close(only_id);
        assert_eq!(model.panes().len(), 1, "a lone root pane should be kept");
        assert_eq!(model.panes()[0].kind, PaneKind::Terminal);
        assert_eq!(revealed, None);
    }

    // MARK: - addPaneIfAbsent

    /// Port of `testAddPaneIfAbsentDoesNotDuplicate`.
    #[test]
    fn add_pane_if_absent_does_not_duplicate() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        model.add_pane_if_absent(PaneKind::Terminal, "dup");
        assert_eq!(model.panes().len(), 1);

        model.add_pane_if_absent(PaneKind::Files, "f");
        assert_eq!(model.panes().len(), 2);
        assert_eq!(
            model
                .panes()
                .iter()
                .filter(|p| p.kind == PaneKind::Files)
                .count(),
            1
        );

        model.add_pane_if_absent(PaneKind::Files, "f2");
        assert_eq!(model.panes().len(), 2);
        assert_eq!(
            model
                .panes()
                .iter()
                .filter(|p| p.kind == PaneKind::Files)
                .count(),
            1
        );
    }

    // MARK: - snapshot / apply (serialization round trip)

    /// Port of `testSnapshotRoundTripPreservesLayout`.
    #[test]
    fn snapshot_round_trip_preserves_layout() {
        let original = PaneTilingModel::default_layout();
        let layout = original.snapshot();

        let restored = PaneTilingModel::model_from(&layout);
        assert!(restored.is_some());
        let restored = restored.unwrap();
        assert_eq!(
            restored.panes().iter().map(|p| p.kind).collect::<Vec<_>>(),
            original.panes().iter().map(|p| p.kind).collect::<Vec<_>>()
        );
        assert_eq!(restored.root.orientation, TileOrientation::Vertical);
        approx_eq(restored.root.ratio, 0.55);
    }

    /// Port of `testApplyReplacesTree`.
    #[test]
    fn apply_replaces_tree() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        let target = PaneTilingModel::default_layout().snapshot();
        model.apply(&target);
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

    /// Port of `testModelFromInvalidLayoutReturnsNil`.
    #[test]
    fn model_from_invalid_layout_returns_none() {
        let broken = TileLayout {
            orientation: Some("horizontal".to_string()),
            ratio: Some(0.5),
            children: Some(vec![]),
            ..Default::default()
        };
        assert!(PaneTilingModel::model_from(&broken).is_none());
    }

    /// Port of `testResetToDefaultRestoresFourPanes`.
    #[test]
    fn reset_to_default_restores_four_panes() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        model.add_pane(PaneItem::new(PaneKind::Files, "f"));
        model.reset_to_default();
        assert_eq!(model.panes().len(), 4);
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

    // MARK: - tabs (center-drop merge / edge-drop split-out / per-tab close)

    /// Port of `testMoveCenterMergesIntoTabGroup`.
    #[test]
    fn move_center_merges_into_tab_group() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        model.add_pane(PaneItem::new(PaneKind::Files, "f"));
        let terminal_id = model.panes()[0].id;
        let files_id = model.panes()[1].id;

        model.move_pane(terminal_id, files_id, DropEdge::Center);

        assert_eq!(
            model.panes().len(),
            2,
            "tab merge shouldn't change total pane count"
        );
        assert!(
            model.root.is_leaf(),
            "the two leaves should collapse into one tab group"
        );
        assert_eq!(
            model.root.panes.iter().map(|p| p.kind).collect::<Vec<_>>(),
            vec![PaneKind::Files, PaneKind::Terminal]
        );
        assert_eq!(
            model.root.selected_pane().map(|p| p.id),
            Some(terminal_id),
            "the merged tab should be selected"
        );
    }

    /// Port of `testMoveEdgeSplitsTabOutOfGroup`.
    #[test]
    fn move_edge_splits_tab_out_of_group() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        model.add_pane(PaneItem::new(PaneKind::Files, "f"));
        let terminal_id = model.panes()[0].id;
        let files_id = model.panes()[1].id;
        model.move_pane(terminal_id, files_id, DropEdge::Center); // merge first

        model.move_pane(terminal_id, files_id, DropEdge::Right); // split out to the right

        assert!(!model.root.is_leaf());
        assert_eq!(model.panes().len(), 2);
        assert_eq!(
            model.root.children[0].selected_pane().map(|p| p.kind),
            Some(PaneKind::Files)
        );
        assert_eq!(
            model.root.children[1].selected_pane().map(|p| p.kind),
            Some(PaneKind::Terminal)
        );
    }

    /// Port of `testCloseTabInGroupKeepsSiblingTabs`.
    #[test]
    fn close_tab_in_group_keeps_sibling_tabs() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        model.add_pane(PaneItem::new(PaneKind::Files, "f"));
        let terminal_id = model.panes()[0].id;
        let files_id = model.panes()[1].id;
        model.move_pane(terminal_id, files_id, DropEdge::Center);

        let revealed = model.close(terminal_id);

        assert_eq!(model.panes().len(), 1);
        assert_eq!(model.panes()[0].kind, PaneKind::Files);
        assert_eq!(
            revealed,
            Some(files_id),
            "closing should reveal the tab that becomes front-facing"
        );
    }

    // MARK: - tab group serialization / restore

    /// Port of `testTabGroupSnapshotRoundTrip`.
    #[test]
    fn tab_group_snapshot_round_trip() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        model.add_pane(PaneItem::new(PaneKind::Terminal, "t2"));
        let a = model.panes()[0].id;
        let b = model.panes()[1].id;
        model.move_pane(a, b, DropEdge::Center);
        // Mirrors the Swift test's `model.panes[0].agentSessionID = ...`:
        // `panes` is recomputed after the move, so index 0 is whatever tab
        // now sits first in the merged group (the merge target `b`'s pane,
        // since `a` gets *appended* -- not necessarily `a` itself).
        model.root.panes[0].agent_session_id = Some("sid-1".to_string());
        model.root.panes[0].agent_transcript_path = Some("/tmp/t1.jsonl".to_string());

        let restored = PaneTilingModel::model_from(&model.snapshot());

        assert!(restored.is_some());
        let restored = restored.unwrap();
        assert!(restored.root.is_leaf());
        assert_eq!(restored.root.panes.len(), 2);
        assert_eq!(
            restored.root.panes[0].agent_session_id.as_deref(),
            Some("sid-1")
        );
        assert_eq!(
            restored.root.panes[0].agent_transcript_path.as_deref(),
            Some("/tmp/t1.jsonl")
        );
        assert_eq!(restored.root.selected_index, model.root.selected_index);
    }

    /// Port of `testSingleTabEncodesLegacyFormatAndDecodesIt`.
    #[test]
    fn single_tab_encodes_legacy_format_and_decodes_it() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        let only_id = model.panes()[0].id;
        model.pane_mut(only_id).unwrap().agent_session_id = Some("sid-9".to_string());
        let layout = model.snapshot();
        assert_eq!(
            layout.pane_kind.as_deref(),
            Some("terminal"),
            "a single tab should write the legacy shape"
        );
        assert!(layout.panes.is_none());
        assert_eq!(layout.pane_agent_session_id.as_deref(), Some("sid-9"));

        // Legacy data (pre-tabs JSON shape) should also restore.
        let legacy = TileLayout {
            pane_kind: Some("files".to_string()),
            pane_title: Some("変更".to_string()),
            ..Default::default()
        };
        let restored = PaneTilingModel::model_from(&legacy).unwrap();
        assert_eq!(
            restored.root.selected_pane().map(|p| p.kind),
            Some(PaneKind::Files)
        );
        assert_eq!(
            restored.root.selected_pane().map(|p| p.title.as_str()),
            Some("変更")
        );
    }

    // MARK: - tab color (第10波 パーソナライズ)

    /// A single-tab leaf's color rides the legacy shape's new `paneColor`
    /// key: written only when set (`None` -> key omitted, so pre-第10波
    /// JSON stays byte-identical -- see `PanePayload::color`'s doc
    /// comment), and restored on decode.
    #[test]
    fn single_tab_color_round_trips_through_the_legacy_shape_and_json() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        let only_id = model.panes()[0].id;

        // No color -> no key in the serialized JSON at all.
        let plain = model.snapshot();
        assert_eq!(plain.pane_color, None);
        assert!(!plain.to_json().contains("paneColor"));

        model.pane_mut(only_id).unwrap().color = Some("#d0ff00".to_string());
        let layout = model.snapshot();
        assert_eq!(layout.pane_color.as_deref(), Some("#d0ff00"));
        let json = layout.to_json();
        assert!(json.contains("\"paneColor\":\"#d0ff00\""));

        let reparsed = TileLayout::from_json(&json).unwrap();
        let restored = PaneTilingModel::model_from(&reparsed).unwrap();
        assert_eq!(
            restored
                .root
                .selected_pane()
                .and_then(|p| p.color.as_deref()),
            Some("#d0ff00")
        );
    }

    /// A tab group's per-tab colors ride `PanePayload::color` (the new
    /// `color` key), independently per tab, through snapshot -> JSON ->
    /// decode.
    #[test]
    fn tab_group_colors_round_trip_per_tab() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        model.add_pane(PaneItem::new(PaneKind::Terminal, "t2"));
        let first = model.panes()[0].id;
        model.pane_mut(first).unwrap().color = Some("#ff9f0a".to_string());
        // Second tab deliberately left color-less.

        let json = model.snapshot().to_json();
        let restored = PaneTilingModel::model_from(&TileLayout::from_json(&json).unwrap()).unwrap();
        let colors: Vec<Option<&str>> = restored
            .panes()
            .iter()
            .map(|p| p.color.as_deref())
            .collect();
        assert_eq!(colors, vec![Some("#ff9f0a"), None]);
    }

    /// Layout JSON written before this wave (no color keys anywhere)
    /// decodes with every tab color `None` -- `serde`'s missing-optional
    /// default, same contract as every other optional key.
    #[test]
    fn pre_color_json_decodes_with_no_tab_colors() {
        let legacy =
            TileLayout::from_json(r#"{"paneKind":"terminal","paneTitle":"端末"}"#).unwrap();
        assert_eq!(legacy.pane_color, None);
        let restored = PaneTilingModel::model_from(&legacy).unwrap();
        assert_eq!(
            restored.root.selected_pane().and_then(|p| p.color.clone()),
            None
        );
    }

    /// `stripping_agent_sessions` (preset save) keeps tab colors -- a color
    /// is part of the layout's visual shape, like a title (see the method's
    /// doc comment).
    #[test]
    fn stripping_agent_sessions_keeps_tab_colors() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        model.add_pane(PaneItem::with_agent_session(
            PaneKind::Terminal,
            "t2",
            "sid-2",
            "/tmp/x",
        ));
        let first = model.panes()[0].id;
        model.pane_mut(first).unwrap().color = Some("#30d158".to_string());

        let stripped = model.snapshot().stripping_agent_sessions();
        let restored = PaneTilingModel::model_from(&stripped).unwrap();
        assert_eq!(
            restored
                .panes()
                .iter()
                .map(|p| p.color.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("#30d158"), None]
        );
        assert!(restored
            .panes()
            .iter()
            .all(|p| p.agent_session_id.is_none()));
    }

    /// Port of `testStrippingAgentSessionsRemovesIDs`.
    #[test]
    fn stripping_agent_sessions_removes_ids() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        model.add_pane(PaneItem::with_agent_session(
            PaneKind::Terminal,
            "t2",
            "sid-2",
            "/tmp/x",
        ));
        let first_id = model.panes()[0].id;
        model.pane_mut(first_id).unwrap().agent_session_id = Some("sid-1".to_string());

        let stripped = model.snapshot().stripping_agent_sessions();
        let restored = PaneTilingModel::model_from(&stripped).unwrap();

        assert_eq!(restored.panes().len(), 2);
        assert!(restored
            .panes()
            .iter()
            .all(|p| p.agent_session_id.is_none()));
        assert!(restored
            .panes()
            .iter()
            .all(|p| p.agent_transcript_path.is_none()));
    }

    // MARK: - revision

    /// Port of `testMutationBumpsRevisionAndFiresLayoutCallback`.
    #[test]
    fn mutation_bumps_revision_and_fires_layout_callback() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        assert_eq!(model.revision(), 0);

        let call_count = Rc::new(RefCell::new(0));
        let call_count_cb = Rc::clone(&call_count);
        model.on_layout_changed = Some(Box::new(move || {
            *call_count_cb.borrow_mut() += 1;
        }));

        model.add_pane(PaneItem::new(PaneKind::Files, "f"));
        assert_eq!(model.revision(), 1);
        assert_eq!(*call_count.borrow(), 1);

        let leaf_id = model.panes()[0].id;
        model.split(
            leaf_id,
            TileOrientation::Vertical,
            PaneItem::new(PaneKind::Diff, "Diff"),
        );
        assert_eq!(model.revision(), 2);
        assert_eq!(*call_count.borrow(), 2);
    }

    // MARK: - bonus coverage (not part of the 22 ported Swift tests: these
    // exercise `PaneTilingActions`/`record_agent_session`, which
    // `PaneTilingTests.swift` doesn't cover directly)

    #[derive(Default)]
    struct MockCoordinator {
        pending_focus_pane_id: Option<PaneId>,
        sent: Vec<(PaneId, String)>,
    }

    impl PaneTilingActions for MockCoordinator {
        fn pending_focus_pane_id(&self) -> Option<PaneId> {
            self.pending_focus_pane_id
        }
        fn set_pending_focus_pane_id(&mut self, pane_id: Option<PaneId>) {
            self.pending_focus_pane_id = pane_id;
        }
        fn schedule_send(&mut self, pane_id: PaneId, text: String) {
            self.sent.push((pane_id, text));
        }
    }

    #[test]
    fn launch_in_new_terminal_adds_pane_focuses_and_sends_command() {
        let mut model = make_single_pane_model(PaneKind::Files);
        let coordinator = MockCoordinator::default();
        model.coordinator = Some(Box::new(coordinator));

        model.launch_in_new_terminal("Claude", "claude --resume");

        assert_eq!(model.panes().len(), 2);
        let terminal_id = model
            .panes()
            .iter()
            .find(|p| p.kind == PaneKind::Terminal)
            .map(|p| p.id)
            .unwrap();
        let coordinator = model.coordinator.as_ref().unwrap().pending_focus_pane_id();
        assert_eq!(coordinator, Some(terminal_id));
    }

    #[test]
    fn send_to_existing_terminal_returns_false_without_a_terminal_pane() {
        let mut model = make_single_pane_model(PaneKind::Files);
        assert!(!model.send_to_existing_terminal("echo hi"));
    }

    #[test]
    fn record_agent_session_ignores_non_terminal_panes() {
        let mut model = make_single_pane_model(PaneKind::Files);
        let pane_id = model.panes()[0].id;
        model.record_agent_session("sid", pane_id, Some("/tmp/x".to_string()));
        assert!(model.panes()[0].agent_session_id.is_none());
    }

    #[test]
    fn record_agent_session_sets_fields_and_does_not_bump_revision() {
        let mut model = make_single_pane_model(PaneKind::Terminal);
        let pane_id = model.panes()[0].id;
        let call_count = Rc::new(RefCell::new(0));
        let call_count_cb = Rc::clone(&call_count);
        model.on_layout_changed = Some(Box::new(move || {
            *call_count_cb.borrow_mut() += 1;
        }));

        model.record_agent_session("sid-1", pane_id, Some("/tmp/x.jsonl".to_string()));

        assert_eq!(
            model.revision(),
            0,
            "record_agent_session must not bump revision"
        );
        assert_eq!(*call_count.borrow(), 1);
        assert_eq!(model.panes()[0].agent_session_id.as_deref(), Some("sid-1"));
    }

    // MARK: - PaneItem::is_resumable (resume-at-restore gate)

    #[test]
    fn is_resumable_false_without_a_session_id() {
        let pane = PaneItem::new(PaneKind::Terminal, "t");
        assert!(!pane.is_resumable(true));
        assert!(!pane.is_resumable(false));
    }

    #[test]
    fn is_resumable_false_for_an_empty_session_id() {
        let pane = PaneItem::with_agent_session(PaneKind::Terminal, "t", "", "/tmp/x.jsonl");
        assert!(!pane.is_resumable(true));
    }

    #[test]
    fn is_resumable_true_with_session_id_and_no_recorded_transcript_path() {
        // Old data with no transcript path recorded is tried as before
        // (docs/hooks-protocol.md §6: "パス未記録（旧データ）は従来どおり試す").
        let mut pane = PaneItem::with_agent_session(PaneKind::Terminal, "t", "sid-1", "/tmp/x");
        pane.agent_transcript_path = None;
        assert!(pane.is_resumable(false));
    }

    #[test]
    fn is_resumable_requires_transcript_to_exist_when_a_path_is_recorded() {
        let pane =
            PaneItem::with_agent_session(PaneKind::Terminal, "t", "sid-1", "/tmp/gone.jsonl");
        assert!(
            !pane.is_resumable(false),
            "missing transcript blocks resume"
        );
        assert!(pane.is_resumable(true), "existing transcript allows resume");
    }

    // MARK: - drop_edge_for_point

    #[test]
    fn drop_edge_for_point_center_is_the_inner_50_percent() {
        assert_eq!(
            drop_edge_for_point(100.0, 100.0, 50.0, 50.0),
            DropEdge::Center
        );
        // Just inside the 25%/75% boundaries on every side.
        assert_eq!(
            drop_edge_for_point(100.0, 100.0, 26.0, 26.0),
            DropEdge::Center
        );
        assert_eq!(
            drop_edge_for_point(100.0, 100.0, 74.0, 74.0),
            DropEdge::Center
        );
    }

    #[test]
    fn drop_edge_for_point_left_and_right_are_the_outer_25_percent_columns() {
        assert_eq!(drop_edge_for_point(100.0, 100.0, 0.0, 50.0), DropEdge::Left);
        assert_eq!(
            drop_edge_for_point(100.0, 100.0, 24.0, 50.0),
            DropEdge::Left
        );
        assert_eq!(
            drop_edge_for_point(100.0, 100.0, 76.0, 50.0),
            DropEdge::Right
        );
        assert_eq!(
            drop_edge_for_point(100.0, 100.0, 100.0, 50.0),
            DropEdge::Right
        );
    }

    #[test]
    fn drop_edge_for_point_top_and_bottom_are_checked_only_within_the_center_column() {
        // Left/right take priority over top/bottom -- a corner point resolves
        // to the horizontal edge, matching `PaneFrameView.edge(at:)`'s
        // `if rx < 0.25 { .left } else if rx > 0.75 { .right } else if ...`
        // check ordering.
        assert_eq!(drop_edge_for_point(100.0, 100.0, 50.0, 0.0), DropEdge::Top);
        assert_eq!(drop_edge_for_point(100.0, 100.0, 50.0, 24.0), DropEdge::Top);
        assert_eq!(
            drop_edge_for_point(100.0, 100.0, 50.0, 76.0),
            DropEdge::Bottom
        );
        assert_eq!(
            drop_edge_for_point(100.0, 100.0, 50.0, 100.0),
            DropEdge::Bottom
        );
        assert_eq!(
            drop_edge_for_point(100.0, 100.0, 0.0, 0.0),
            DropEdge::Left,
            "top-left corner resolves to left, not top (x-axis checked first)"
        );
    }

    #[test]
    fn drop_edge_for_point_is_not_symmetric_with_appkits_flipped_convention() {
        // gpui's Y increases downward: a point near y=0 (top of the rect in
        // screen terms) is `Top`, not `Bottom` -- the opposite of what
        // `PaneFrameView.edge(at:)` returned for the same fraction under
        // AppKit's flipped view coordinates. See this function's doc comment.
        assert_eq!(drop_edge_for_point(100.0, 100.0, 50.0, 5.0), DropEdge::Top);
    }

    #[test]
    fn drop_edge_for_point_non_square_rectangle_uses_each_axis_own_fraction() {
        // A wide, short rectangle: the same absolute x that would be "left"
        // in a square rect is well within the center 50% here.
        assert_eq!(
            drop_edge_for_point(400.0, 100.0, 150.0, 50.0),
            DropEdge::Center
        );
        assert_eq!(
            drop_edge_for_point(400.0, 100.0, 50.0, 50.0),
            DropEdge::Left
        );
    }

    #[test]
    fn drop_edge_for_point_degenerate_rectangle_falls_back_to_center() {
        assert_eq!(drop_edge_for_point(0.0, 100.0, 0.0, 50.0), DropEdge::Center);
        assert_eq!(drop_edge_for_point(100.0, 0.0, 50.0, 0.0), DropEdge::Center);
        assert_eq!(
            drop_edge_for_point(-1.0, 100.0, 0.0, 50.0),
            DropEdge::Center
        );
    }

    // MARK: - split ratio (interactive divider drag-resize, W5j #2). Not
    // ported Swift tests -- the Swift source never routed ratio changes
    // through the model at all (see `PaneTilingModel::set_split_ratio`'s
    // doc comment), so there is no oracle behavior to match here; this is
    // new Rust-only surface.

    #[test]
    fn set_ratio_stores_an_in_range_value_unchanged() {
        let mut node = TileNode::split(TileOrientation::Horizontal, 0.5, vec![]);
        assert_eq!(node.set_ratio(0.3), 0.3);
        approx_eq(node.ratio, 0.3);
    }

    #[test]
    fn set_ratio_clamps_below_the_minimum() {
        let mut node = TileNode::split(TileOrientation::Horizontal, 0.5, vec![]);
        assert_eq!(node.set_ratio(-1.0), MIN_SPLIT_RATIO);
        assert_eq!(node.set_ratio(0.0), MIN_SPLIT_RATIO);
        approx_eq(node.ratio, MIN_SPLIT_RATIO);
    }

    #[test]
    fn set_ratio_clamps_above_the_maximum() {
        let mut node = TileNode::split(TileOrientation::Horizontal, 0.5, vec![]);
        assert_eq!(node.set_ratio(2.0), MAX_SPLIT_RATIO);
        assert_eq!(node.set_ratio(1.0), MAX_SPLIT_RATIO);
        approx_eq(node.ratio, MAX_SPLIT_RATIO);
    }

    #[test]
    fn set_ratio_ignores_non_finite_input_and_keeps_the_prior_value() {
        let mut node = TileNode::split(TileOrientation::Horizontal, 0.42, vec![]);
        assert_eq!(node.set_ratio(f64::NAN), 0.42);
        assert_eq!(node.set_ratio(f64::INFINITY), 0.42);
        assert_eq!(node.set_ratio(f64::NEG_INFINITY), 0.42);
        approx_eq(node.ratio, 0.42);
    }

    #[test]
    fn find_node_mut_locates_the_root_split_by_its_own_node_id() {
        let mut model = PaneTilingModel::default_layout();
        let split_id = model.root.id;
        let found = model
            .root
            .find_node_mut(split_id)
            .expect("root finds itself");
        assert_eq!(found.id, split_id);
    }

    #[test]
    fn find_node_mut_locates_a_leaf_by_its_own_node_id() {
        let mut model = PaneTilingModel::default_layout();
        let leaf_node_id = model.root.leaves()[0].id;
        let found = model
            .root
            .find_node_mut(leaf_node_id)
            .expect("leaf's own id resolves via find_node_mut");
        assert!(found.is_leaf());
    }

    #[test]
    fn find_node_mut_unknown_id_returns_none() {
        let mut model = PaneTilingModel::default_layout();
        let other_tree = PaneTilingModel::default_layout();
        assert!(model.root.find_node_mut(other_tree.root.id).is_none());
    }

    #[test]
    fn set_split_ratio_updates_the_addressed_split_node() {
        let mut model = PaneTilingModel::default_layout();
        let split_id = model.root.id;
        let before_revision = model.revision();
        assert!(model.set_split_ratio(split_id, 0.7));
        approx_eq(model.root.ratio, 0.7);
        // Mirrors the Swift source: a ratio change never bumps `revision`
        // (no tree-structure change, no reconcile needed) -- see
        // `set_split_ratio`'s doc comment.
        assert_eq!(model.revision(), before_revision);
    }

    #[test]
    fn set_split_ratio_clamps_via_the_same_bounds_as_set_ratio() {
        let mut model = PaneTilingModel::default_layout();
        let split_id = model.root.id;
        assert!(model.set_split_ratio(split_id, 10.0));
        approx_eq(model.root.ratio, MAX_SPLIT_RATIO);
    }

    #[test]
    fn set_split_ratio_unknown_node_id_is_a_silent_no_op() {
        let mut model = PaneTilingModel::default_layout();
        let original_ratio = model.root.ratio;
        // A NodeId from an entirely different tree never resolves here.
        let other_tree = PaneTilingModel::default_layout();
        let foreign_id = other_tree.root.id;
        assert!(!model.set_split_ratio(foreign_id, 0.9));
        approx_eq(model.root.ratio, original_ratio);
    }

    #[test]
    fn set_split_ratio_reaches_a_nested_split_not_just_the_root() {
        let mut model = PaneTilingModel::default_layout();
        // `default_layout`'s bottom row (`commits | files_and_diff`) is a
        // split nested one level under the root -- addressing it exercises
        // the recursive `find_node_mut` walk, not just the trivial
        // self-match at the root.
        let bottom_row = &model.root.children[1];
        assert_eq!(bottom_row.orientation, TileOrientation::Horizontal);
        let nested_id = bottom_row.id;
        assert!(model.set_split_ratio(nested_id, 0.6));
        approx_eq(model.root.children[1].ratio, 0.6);
        // The root's own ratio is untouched.
        approx_eq(model.root.ratio, 0.55);
    }
}
