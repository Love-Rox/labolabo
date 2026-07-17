# labolabo-core (Rust)

The Rust cross-platform migration's pure-logic core: a faithful port of
LaboLaboEngine's OS/process/UI-independent logic — parsers and pure
algorithms — from Swift to Rust.

> **Sibling crate — `labolabo-term`.** This workspace also contains
> [`crates/labolabo-term`](crates/labolabo-term/README.md): the cross-platform
> **terminal-session core** (a real PTY via `portable-pty` driving a VT parser,
> producing a UI-independent `GridSnapshot`). Unlike `labolabo-core`'s pure
> parsers, it owns live OS resources and has a pluggable VT backend —
> `backend-alacritty` (default, crates.io-only, keeps the standing `rust` CI
> job green) or `backend-ghostty-vt` (the intended production core, real
> `libghostty-vt`, opt-in because it needs Zig 0.16 + a Ghostty source tree).
> It has its own CI job (`rust-term-ghostty`, `continue-on-error`) and is
> distilled from the `term-poc` spike. See its README for the design.

> **Sibling crate — `labolabo-app`.** This workspace also contains
> [`crates/labolabo-app`](crates/labolabo-app/README.md): the wave-5a
> bootable skeleton of the cross-platform UI — a [gpui](https://www.gpui.rs/)
> binary that renders `labolabo-term`'s `GridSnapshot` in a window (with
> the user's own Ghostty `font-family`/`font-size` settings), routes
> keyboard input to a `TermSession`, and drives a minimal tab bar. It is
> **not** in `default-members` (gpui is a heavy desktop-UI dependency this
> workspace's plain `cargo build`/`test`/`clippy` must not pull in), so build
> and test it explicitly with `-p labolabo-app`; it has its own CI jobs
> (`rust-app` on macOS, `rust-app-linux` on ubuntu — added in wave 7a —,
> `rust-app-windows` on windows-latest — added in wave 7c; see the app
> README's "Linux (wave 7a)"/"Windows (wave 7c)" sections for system deps,
> packaging, and what is/isn't verified there). See its README for
> design/scope/TODOs.

As of wave 4c, "pure-logic" no longer means "no I/O": `store` (ported from
`LaboLaboStore`) is real, fallible SQLite persistence, not a parser or an
in-memory model. It's still in-scope for this crate (the porting brief
explicitly puts it here rather than a new crate — see the wave 4c section
below) because it's still OS/UI-framework-independent, just no longer
side-effect-free.

## Wave 1 (`Sources/LaboLaboEngine/Git/`, no runtime deps)

| Swift source | Rust module |
|---|---|
| `GitModels.swift` | `crates/labolabo-core/src/git_models.rs` |
| `PorcelainStatusParser.swift` | `crates/labolabo-core/src/porcelain.rs` |
| `UnifiedDiffParser.swift` | `crates/labolabo-core/src/unified_diff.rs` |

## Wave 2 (commit graph, worktree list, agent status/usage)

| Swift source | Rust module | Golden fixtures? |
|---|---|---|
| `Git/CommitGraph.swift` (pure `CommitGraphLayout.build` only) | `commit_graph.rs` | no — see below |
| `Git/Worktree.swift` | `worktree.rs` | yes |
| `Agent/TranscriptUsage.swift` | `transcript_usage.rs` | yes |
| `Agent/AgentStatus.swift` | `agent_status.rs` | no — see below |
| `Agent/AgentEventParser.swift` | `agent_event_parser.rs` | yes |
| `Git/CrossSessionConflicts.swift` | `cross_session_conflicts.rs` | no — see below |
| `Update/ReleaseVersion.swift` | `release_version.rs` | no — see below |

Per the porting brief, the three **pure-algorithm** modules
(`commit_graph`, `cross_session_conflicts`, `release_version`) carry only
unit tests ported 1:1 from their Swift XCTest suites — no golden fixtures.
`agent_status.rs` is a thin enum-mapping module folded into
`agent_event_parser.rs`'s golden coverage (every `agent_event` fixture
exercises `AgentStatus::from_hook_event` too) rather than getting its own
fixture set.

`commit_graph.rs` ports only the pure `CommitGraphLayout.build(_:)`
function and its result types. The Swift file's
`GitEngine.commitGraph(worktree:limit:)` extension shells out to `git log`
via `GitRunner` — process execution, not pure logic, and out of scope here
(confirmed unlinkable standalone: `nm -g` on `CommitGraph.swift.o` shows an
undefined reference to `GitRunner.run`, whereas `Worktree.swift.o`,
`TranscriptUsage.swift.o`, `AgentStatus.swift.o`, and
`AgentEventParser.swift.o` only reference Foundation/stdlib symbols, which
is why only those four could be wired into the golden-oracle script).

`transcript_usage.rs` and `agent_event_parser.rs` need real JSON parsing to
faithfully reproduce Foundation's `JSONSerialization` + `as? T` bridging
behavior, so `serde_json` is a **runtime** dependency starting this wave
(wave 1's parsers needed none).

Everything else in `LaboLaboEngine` (process execution, git plumbing that
shells out, file watching, the `AgentStatusBus`/`AgentEventTransport`
socket-transport layer, persistence, ...) remains out of scope.

## Wave 3 (tiling/tab tree model)

| Swift source | Rust module | Golden fixtures? |
|---|---|---|
| `app/Sources/PaneTilingModel.swift` | `tiling.rs` | yes (separate mechanism, see below) |

This is the first ported module that lives in the **app target**
(`app/Sources/`), not `LaboLaboEngine` — `PaneTilingModel` is the tile/tab
tree (`TileNode`/`PaneItem`/`PaneTilingModel`) behind one session's
terminal + changed-files + diff + commit-history layout, plus its
persisted-JSON shape (`TileLayout`/`PanePayload`/`LayoutPreset`).

Unlike every wave-1/2 module, `TileLayout`/`PanePayload` are not test-only
JSON views: they are the app's actual `Codable` DTOs, round-tripped through
`JSONEncoder`/`JSONDecoder` to persist a session's layout (GRDB
`appState.paneLayout` column) and named layout presets, so `serde`'s
`derive` feature becomes a **runtime** dependency starting this wave (wave
2 only needed `serde_json` itself, for hand-rolled parsing).

Because existing users already have layouts on disk in whatever shape
Swift's `JSONEncoder` wrote them in, `tiling.rs` documents (and its golden
test enforces) an unusually detailed JSON-compatibility contract: exact key
spellings, the legacy-single-tab/`panes`-tab-group backward-compat split,
the omitted-vs-`null` rule, `/`-escaping, and float formatting (no
trailing `.0` for integral `ratio` values) — all verified empirically
against a real `JSONEncoder`, not assumed. It also documents why object key
**order** is deliberately *not* matched: empirically, `JSONEncoder`'s key
order for a `Codable` struct is not stable even across repeated runs of the
same Swift process (confirmed by encoding the same value four times in
four separate `swift` invocations and getting four different orders), so
"byte-identical to Swift's output" isn't a coherent target for order in the
first place. See `src/tiling.rs`'s module doc comment for the full writeup.

Its golden fixtures (`fixtures/tiling/*.json`) come from a **separate**
oracle mechanism from `fixtures/generate.swift`: since `PaneTilingModel`
lives in the app target, it isn't reachable through `generate.swift`'s
`LaboLaboEngine`-object-file-linking trick. Instead, a disposable
`swiftc`-compiled driver (`main.swift`, not checked in) is compiled
*together* with the real `app/Sources/PaneTilingModel.swift` and calls the
real `JSONEncoder().encode(_:)` directly — see `tests/tiling_golden.rs`'s
module doc comment for the exact regeneration command. One fixture,
`legacy_old_format.json`, is hand-authored instead (it represents
genuinely pre-tab-feature persisted data that today's
`PaneTilingModel.swift` can no longer produce, by construction).

`tiling`'s ported-1:1 behavior unit tests (all 22 of
`Tests/AppUnitTests/PaneTilingTests.swift`) live inside `src/tiling.rs`
itself (`#[cfg(test)] mod tests`), same convention as wave 1/2's parser
modules — only the JSON golden coverage gets its own top-level
`tests/tiling_golden.rs`, per the module's `Formatter`-heavy
compatibility contract needing more elaborate fixtures/assertions than fit
comfortably inline.

Design translations from the Swift source's `@MainActor @Observable`
reference-type tree (`TileNode`/`PaneItem`/`PaneTilingModel` are classes
mutated in place through live object references) to Rust's owned-tree
struct model, plus the deliberate simplification of
`recordAgentSession`'s `UUID`-from-`String` parsing, are documented in
`src/tiling.rs`'s module doc comment.

## Wave 4b (hooks bus + forwarder)

| Swift source | Rust module |
|---|---|
| `Sources/LaboLaboEngine/Agent/AgentStatusBus.swift` (`AgentEventTransport`, `UnixSocketEventTransport`, `AgentStatusBus`) | `src/hooks.rs` |
| `app/Sources/HookForwarder.swift` | `src/hooks.rs` (`forward_hook`) + `src/bin/labolabo-hook.rs` |

Cross-checked directly against `docs/hooks-protocol.md` (the canonical wire
spec, checked in at the repo root) and both Swift sources above — no
divergence found, same as wave 2's `agent_event_parser`.

Unlike every prior wave, this one ports **process/socket infrastructure**,
not pure logic — it was explicitly out of scope through waves 1-3 (see
`agent_event_parser.rs`'s module doc comment). The port is faithful to
observable behavior (1 connection = 1 event framing, bind/chmod/unlink
sequencing, the `LABOLABO_PANE` -> `labolabo_pane_id` annotation rule) while
taking small, deliberately-documented liberties with non-load-bearing
implementation details (e.g. avoiding the double `close(2)` the Swift
`stop()`/`runServer()` pair does on the listening fd) — see `src/hooks.rs`'s
module doc comment and the `UnixSocketEventTransport` struct doc comment for
the specifics.

`AgentStatusBus` here does **not** hop to a main-thread dispatch queue the
way the Swift version does (`DispatchQueue.main.async`) — that's a UI-layer
concern with no analog in this OS/UI-independent core yet; the registered
`on_event` callback runs directly on whatever thread the transport's
`on_message` fires on, and marshaling to a UI thread (if one exists) is left
to the caller, documented explicitly on `AgentStatusBus::start`.

The AF_UNIX transport (`UnixSocketEventTransport`) is `#[cfg(unix)]`;
since the Windows core wave there is also a `#[cfg(windows)]` Named Pipe
transport (`NamedPipeEventTransport`, docs/hooks-protocol.md §4.2) and the
forwarder (`forward_hook`, `src/bin/labolabo-hook.rs`) is
`#[cfg(any(unix, windows))]` — see the "Windows core wave" section below.
Wave 4b introduced the crate's first genuinely platform-specific code and
its first target-specific dependency: `libc` (unix-only,
`[target.'cfg(unix)'.dependencies]`), needed for `shutdown(2)` on a raw fd
to unblock a blocked `accept()` call from another thread when `stop()` is
called — `std::os::unix::net::UnixListener` exposes no such method.

Tests:

- `src/hooks.rs`'s `#[cfg(test)] mod tests`: `annotate_pane`'s three
  scenarios (LABOLABO_PANE present/absent, non-JSON stdin) as pure unit
  tests, plus a from-scratch "transport injection contract" test (a
  hand-rolled mock `AgentEventTransport`, no real socket) proving
  `AgentStatusBus::with_transport` correctly wires `onMessage` through
  `agent_event_parser::parse` to `on_event` and calls `start`/`stop`
  exactly once each — not ported from Swift (the Swift suite always uses
  the real `UnixSocketEventTransport`), added because the DI seam is a
  genuine design point of this port.
- `src/hooks.rs`'s `#[cfg(all(test, any(unix, windows)))] mod
  bus_round_trip_tests`: the real transport round-trip, ported 1:1 from all
  6 tests in `Tests/LaboLaboEngineTests/AgentStatusBusTests.swift` (a real
  client connects and sends one payload per connection; `on_event`
  fires/doesn't fire with the right `AgentStatusEvent`). The test bodies
  are transport-agnostic; per-OS helpers make the same assertions run
  against the AF_UNIX transport on macOS/Linux and the Named Pipe
  transport on Windows CI.
- `tests/labolabo_hook_bin.rs`: one end-to-end test that spawns the actual
  compiled `labolabo-hook` binary (via Cargo's `CARGO_BIN_EXE_labolabo-hook`)
  with `LABOLABO_PANE` set and JSON piped to stdin, and asserts a real
  `UnixListener` receives the pane-id-annotated payload.

## Porting principle: faithful port, not a rewrite

The Swift implementation is the executable spec. Every observable behavior —
including edge cases that look like bugs — is preserved rather than
"improved," because other code (the Swift app today, eventually a Rust UI)
depends on the exact current behavior. Idiomatic Rust translation (`Option`,
`Result`-free error handling via `Option`, iterators, `match`) is fine as
long as the *outputs* for any given input are identical.

Notable edge cases carried over faithfully (see doc comments at the call
sites and the corresponding tests for detail):

- **`GitStatus::is_dirty`** is `true` whenever *any* entry's kind isn't
  `.ignored` — this includes untracked-only status, not just staged/unstaged
  changes.
- **`Change::from_porcelain`** silently falls back to `Unmodified` for any
  unrecognized porcelain XY character instead of erroring.
- Malformed/truncated porcelain records (too few space-separated fields) are
  silently dropped rather than surfaced as errors; parsing continues with
  the next token.
- A rename/copy (`2 ...`) record always consumes the *next* NUL token as its
  original path, even if the record itself fails to parse — so a malformed
  rename record can still "eat" an unrelated following token.
- `String.dropFirst(n)` in Swift clamps instead of panicking when the string
  is shorter than `n`; the Rust port's `drop_first_chars` helper
  (`src/util.rs`) replicates that clamping rather than panicking or
  returning an `Option`.
- **The unified-diff parser's line-prefix checks (`"--- "`, `"+++ "`,
  `"new file mode"`, `"rename from "`, `"@@"`, `"Binary files "`, ...) run
  unconditionally against every line, including lines already inside an
  open hunk.** The Swift source is a flat, state-unaware prefix scanner, not
  a proper per-region parser. Concretely: a *deleted* content line whose own
  text begins with `"-- "` renders as a raw diff line starting with
  `"--- "` (deletion marker `-` + `"-- "`...), which gets misdetected as the
  `"--- a/path"` header line instead of a hunk line — the line silently
  disappears from `hunk.lines` and `FileDiff.oldPath` gets corrupted to a
  bogus value parsed out of it. See
  `crates/labolabo-core/src/unified_diff.rs`'s
  `quirk_deletion_line_starting_with_dash_dash_dash_is_misparsed_as_old_path_header`
  test and `fixtures/inputs/diff/quirk_dash_dash_dash_deletion_line.diff`.
- A **pure rename with no content change** (`git diff -M` at 100%
  similarity) emits only `rename from`/`rename to` lines — no `--- `/`+++ `
  lines and no hunk at all. `FileDiff.oldPath`/`newPath` still get set (from
  the rename lines), but `hunks` stays empty.
- Line counting in a hunk: `additions`/`deletions` are literally "how many
  lines of that kind ended up in `hunks[*].lines`" — they are **not**
  cross-checked against the hunk header's declared `oldCount`/`newCount`. If
  a line is lost to the quirk above, the header's counts and the actual
  line counts can disagree; the parser does not validate this.
- `raw.split(separator: "\n", omittingEmptySubsequences: false)` (diff
  input) and `raw.split(separator: "\u{0}", omittingEmptySubsequences:
  true)` (porcelain input) have different empty-subsequence behavior in
  Swift, and the Rust port matches each individually (`raw.split('\n')` vs.
  `raw.split('\0').filter(|s| !s.is_empty())`).

Wave 2 edge cases:

- **`transcript_usage::as_int` NSNumber-bridging quirk**: Swift's
  `(u["input_tokens"] as? Int) ?? 0` is not "parse a JSON integer" — it was
  empirically verified (not assumed; see the doc comment on `as_int`) that
  `JSONSerialization` + `as? Int` also bridges whole-number JSON floats
  (`100.0` -> `100`, not the `0` fallback) *and* JSON booleans (`true` ->
  `1`, `false` -> `0`). `serde_json::Number::as_i64()` does **not** do
  either of these (`None` for anything parsed from a float literal, even a
  whole one), so `as_int` reimplements the bridging by hand. See the
  `quirk_*` tests in `transcript_usage.rs` and the
  `whole_number_float_bridges_to_int` / `bool_bridges_to_int_quirk` /
  `fractional_float_and_string_fall_back_to_zero` golden fixtures.
- `TranscriptUsage.parse`'s line splitting (Swift: any `Character` where
  `isNewline` is true, including a lone `\r` and Unicode line separators;
  Rust: plain `\n`) is a deliberate, documented simplification — see the
  doc comment on `transcript_usage::parse` for why it doesn't change
  behavior for real (`\n`-terminated) transcripts.
- `AgentEventParser`/`agent_event_parser::parse`: a non-object top-level
  JSON value (e.g. a bare array) is dropped, matching Swift's
  `try? JSONSerialization.jsonObject(with:) as? [String: Any]` failing the
  cast. Unlike the `Int` bridging above, `as? String` has **no** bridging
  quirks (verified empirically too) — only an actual JSON string matches.

## Workspace layout

```
rust/
  Cargo.toml                        # workspace, resolver = "2"
  crates/labolabo-core/
    Cargo.toml                      # runtime deps: serde_json (wave 2), serde derive (wave 3), libc (unix-only, wave 4b), rusqlite + chrono (wave 4c)
    src/
      lib.rs
      git_models.rs                 # wave 1: port of GitModels.swift + unit tests ported from Swift XCTest
      porcelain.rs                  # wave 1: port of PorcelainStatusParser.swift + unit tests
      unified_diff.rs               # wave 1: port of UnifiedDiffParser.swift + unit tests
      commit_graph.rs              # wave 2: port of CommitGraph.swift's pure layout algorithm + unit tests
      worktree.rs                   # wave 2: port of Worktree.swift + unit test + golden coverage
      transcript_usage.rs           # wave 2: port of TranscriptUsage.swift + unit tests + golden coverage
      agent_status.rs               # wave 2: port of AgentStatus.swift + unit tests
      agent_event_parser.rs         # wave 2: port of AgentEventParser.swift + unit tests + golden coverage
      cross_session_conflicts.rs    # wave 2: port of CrossSessionConflicts.swift + unit tests
      release_version.rs            # wave 2: port of ReleaseVersion.swift + unit tests
      tiling.rs                     # wave 3: port of app/Sources/PaneTilingModel.swift + unit tests
      store/                        # wave 4c: port of Sources/LaboLaboStore/ -- see "Wave 4c" above
        mod.rs
        record.rs                  # port of SessionRecord.swift
        database.rs                # port of SessionDatabase.swift (rusqlite) + unit tests
        datetime.rs                # GRDB `Date` <-> TEXT compatibility (format/parse) + unit tests
        persisting.rs              # port of SessionPersisting.swift (trait)
        data_dir.rs                # port of AppDataDirectory.swift (macOS/Linux/Windows) + unit tests
        error.rs                   # StoreError/StoreResult
      util.rs                       # small string helpers shared by the parsers
      hooks.rs                      # wave 4b: port of AgentStatusBus.swift + HookForwarder.swift + unit tests
      bin/
        labolabo-hook.rs             # wave 4b: thin `labolabo-hook <socket>` forwarder binary
    tests/
      golden.rs                     # golden-oracle test (see below; wave 1/2 modules only)
      tiling_golden.rs               # wave 3: tiling's own golden test (separate oracle mechanism, see below)
      labolabo_hook_bin.rs           # wave 4b: end-to-end test spawning the real labolabo-hook binary
      store_golden.rs                # wave 4c: store's own golden test, against a real-GRDB-written fixture DB (see "Wave 4c" above)
    fixtures/
      generate.swift                # the Swift-side "oracle" generator (see below; wave 1/2 modules only)
      tiling/*.json                 # wave 3: real JSONEncoder output for TileLayout (separate oracle, see below)
      store/fixture.db               # wave 4c: real GRDB-written SQLite fixture (separate oracle, see "Wave 4c" above)
      inputs/
        porcelain/*.txt, *.raw      # git status --porcelain=v2 -z inputs
        diff/*.diff                 # git diff inputs
        worktree/*.txt              # git worktree list --porcelain inputs
        transcript_usage/*.jsonl    # agent transcript (JSONL) inputs
        agent_event/*.txt           # hook-event JSON payloads (`.txt`, not `.json` -- see note below)
      expected/
        porcelain/*.json            # canonical JSON produced by the *Swift* parser
        diff/*.json
        worktree/*.json
        transcript_usage/*.json
        agent_event/*.json
```

`fixtures/inputs/agent_event/*` use a `.txt` extension even though their
content is JSON: `generate.swift`'s `processDirectory` helper skips any
input-directory file whose extension is `.json` (it assumes that's a
leftover *expected*-output file accidentally sitting in `inputs/`), so a
same-extension input file would silently be excluded from generation — hit
this for real while authoring these fixtures (`0 agent_event` fixtures
generated on the first attempt) and renamed them to `.txt` to fix it.

## Golden-oracle testing

Correctness is anchored to the Swift implementation: `tests/golden.rs` reads
every fixture under `fixtures/inputs/{porcelain,diff,worktree,transcript_usage,agent_event}/`,
runs it through the Rust parsers, renders a canonical JSON view, and asserts
it is **byte-identical** to the corresponding file under
`fixtures/expected/{porcelain,diff,worktree,transcript_usage,agent_event}/`
— which was produced by running the *same* fixture files through the real
Swift parsers. `commit_graph`, `cross_session_conflicts`, and
`release_version` (the pure-algorithm modules) are not part of this —
see "Wave 2" above.

Canonical JSON rules (must match on both sides):

- Compact form, no inserted whitespace.
- Object keys in alphabetical order.
  - Rust side: every `*View` struct in `tests/golden.rs` declares its
    fields in alphabetical-by-JSON-key order, because `serde_json::to_string`
    on a struct serializes in field-declaration order (it does not go
    through `serde_json::Value`'s sorted `Map`).
  - Swift side: `fixtures/generate.swift` sorts object keys explicitly
    before rendering.
- Optional/absent fields are **omitted** as a key, never emitted as `null`
  (`#[serde(skip_serializing_if = "Option::is_none")]` on the Rust side;
  the hand-rolled encoder just doesn't add the key on the Swift side).
- Integers as plain base-10; strings with minimal JSON escaping (`"`, `\`,
  and control characters below `0x20`); everything else (including non-ASCII
  UTF-8) passed through unescaped.

### Fixture corpus

`fixtures/inputs/porcelain/` and `fixtures/inputs/diff/` contain:

1. The exact raw inputs from the existing Swift XCTest cases
   (`Tests/LaboLaboEngineTests/PorcelainStatusParserTests.swift` and
   `UnifiedDiffParserTests.swift`), byte-for-byte, so the golden test also
   covers everything those tests cover.
2. Additional hand-authored edge cases the parsers handle but the existing
   Swift tests didn't exercise: empty input, `branch.oid (initial)`,
   unmerged (`u`) conflict entries, ignored (`!`) entries, copy (`C`, not
   just rename) entries, a malformed/truncated ordinary record, an unknown
   leading marker character, a deleted-file diff, a rename with an
   accompanying content hunk, a `\ No newline at end of file` marker,
   multiple hunks in one file, and the `"--- "`-collision quirk described
   above.
3. Real captures from scratch git repositories (`realrepo_*`): mixed
   staged/unstaged/untracked/ignored/renamed/binary status in one
   `git status --porcelain=v2 --branch -z --ignored` run, a real merge
   conflict's `u` line, a real ahead/behind-with-upstream header, and real
   `git diff` / `git diff --cached -M` output covering a new file, a binary
   modification, a text modification, a deletion, and a rename together.
   These repos were built under a scratch directory outside this
   repository and are not part of it — only their captured `git` output is
   checked in as fixtures.

`fixtures/inputs/worktree/` (5 fixtures) covers: the existing Swift test's
three-block scenario (main branch, feature branch, locked+detached with no
trailing separator, exercising end-of-input flush), empty input, a bare
repository, an unknown key (`prunable ...`) interleaved with a `locked
<reason>` line (value ignored, flag still set), and a trailing blank line
after the last block (must not produce a phantom empty entry).

`fixtures/inputs/transcript_usage/` (8 fixtures) covers: the existing
Swift tests' two scenarios (multi-turn sum, non-assistant/malformed lines
ignored), empty input, and five wave-2-specific cases exercising the
`as_int` NSNumber-bridging quirk described above plus a missing-fields and
an empty-model-does-not-overwrite case.

`fixtures/inputs/agent_event/` (8 fixtures) covers: the existing Swift
tests' scenarios (full event, optional fields absent, unknown hook event
dropped, malformed/empty payload dropped, unknown fields ignored) plus two
wave-2-specific cases (non-object top level dropped, a non-string field
silently ignored rather than coerced).

### Regenerating `fixtures/expected/**`

`fixtures/expected/**` must be regenerated any time a fixture is
added/changed *or* the canonical JSON schema changes. It is produced by a
disposable Swift "oracle" script, `fixtures/generate.swift`, that imports
the real `LaboLaboEngine` module and runs the Swift parsers over every file
in `fixtures/inputs/`.

`fixtures/generate.swift` is **not** part of the SwiftPM package graph — no
executable target was added to `Package.swift` (the porting brief
explicitly disallows that, to keep the Swift package's own structure
untouched). Instead it links directly against the already-built object
files for the ported Swift sources (which depend on nothing outside
Foundation — see "Wave 2" above for how this was verified per-file with
`nm -g`) and is compiled as an ordinary one-off `swiftc` binary. From the
repo root:

```sh
# 1. Make sure LaboLaboEngine is built (produces the .o files we link against).
swift build

# 2. Compile the oracle script against those object files.
BUILD=.build/arm64-apple-macosx/debug   # adjust triple if not on arm64 macOS
swiftc -O -I "$BUILD/Modules" rust/crates/labolabo-core/fixtures/generate.swift \
  "$BUILD/LaboLaboEngine.build/GitModels.swift.o" \
  "$BUILD/LaboLaboEngine.build/PorcelainStatusParser.swift.o" \
  "$BUILD/LaboLaboEngine.build/UnifiedDiffParser.swift.o" \
  "$BUILD/LaboLaboEngine.build/Worktree.swift.o" \
  "$BUILD/LaboLaboEngine.build/TranscriptUsage.swift.o" \
  "$BUILD/LaboLaboEngine.build/AgentStatus.swift.o" \
  "$BUILD/LaboLaboEngine.build/AgentEventParser.swift.o" \
  -o /tmp/labolabo_golden_generate

# 3. Run it, pointing at the fixtures directory. It (re)writes every file
#    under fixtures/expected/{porcelain,diff,worktree,transcript_usage,agent_event}/*.json.
/tmp/labolabo_golden_generate rust/crates/labolabo-core/fixtures
```

This leaves zero footprint in `Sources/` or `Tests/` — nothing to add and
then delete before committing. (If this trick ever stops working on some
future toolchain, the brief's documented fallback is to temporarily add a
`#if GOLDEN_EXPORT`-guarded test to `Tests/LaboLaboEngineTests/`, run it via
`swift test --filter`, then delete it before committing — the JSON-shape
logic in `generate.swift` can be pasted in as-is.)

After regenerating, run `cd rust && cargo test` — if a fixture or schema
changed, the affected golden test in `tests/golden.rs` will fail with a
byte-diff-style message naming the mismatched fixture.

## Verification run

```sh
cd rust
cargo test                    # unit tests (ported from Swift XCTest) + golden tests
cargo fmt --check
cargo clippy --all-targets -- -D warnings

cd ..
swift test                    # confirms the Swift side is untouched and still green
```

CI (`.github/workflows/ci.yml`, `rust` job) runs the same three `cargo`
commands on both `ubuntu-latest` and `macos-15`.

## Known scope limits / what's next

- `FileWatcher` remains unported and out of scope. `GitRunner`/`GitEngine`
  (process execution + orchestration) were ported in wave 4a (including the
  thin `git log` wrapper `GitEngine::commit_graph` around
  `commit_graph::build`), the `AgentStatusBus`/`AgentEventTransport`
  socket-transport layer in wave 4b, and persistence (`LaboLaboStore`) in
  wave 4c (`store`) — see the corresponding sections above. The
  settings.local.json hooks-injection app-layer logic
  (`app/Sources/AgentSessionModel.swift`) that creates `/tmp/labolabo` and
  merges/restores the worktree's `.claude/settings.local.json` is still
  unported (app-layer, not engine-layer, same split as the Swift source).
- Golden coverage exists for `porcelain`, `unified_diff`, `worktree`,
  `transcript_usage`, `agent_event_parser`, `tiling`, and `store` (`tiling`
  via its own `tests/tiling_golden.rs`, `store` via its own
  `tests/store_golden.rs` against a real-GRDB-written fixture database —
  neither is `tests/golden.rs`). `commit_graph`, `cross_session_conflicts`,
  and `release_version` are covered by ported unit tests only (no golden
  fixtures), by design — see "Wave 2" above.
- `tiling::PaneTilingActions` is a trait with no production implementation
  yet (no Rust UI layer exists to implement it against) — only a
  test-only mock (`tiling::tests::MockCoordinator`). Likewise
  `store::SessionPersisting` has one conformer (`store::SessionDatabase`)
  and no production call site yet (no Rust UI layer exists to drive it).
- `store` opens a database file directly (`SessionDatabase::open`/
  `open_in_memory`); it does not yet replicate GRDB's `DatabaseQueue`
  single-writer-serialization guarantees under concurrent access from
  multiple threads/processes — not a concern for the single-threaded Rust
  call sites that exist today (there are none yet), but worth revisiting
  once a Rust UI layer actually drives this concurrently.

## Wave 4c (session persistence)

| Swift source | Rust module |
|---|---|
| `Sources/LaboLaboStore/SessionRecord.swift` | `store/record.rs` |
| `Sources/LaboLaboStore/SessionDatabase.swift` | `store/database.rs` (+ `store/datetime.rs` for the `Date` compatibility contract) |
| `Sources/LaboLaboStore/SessionPersisting.swift` | `store/persisting.rs` |
| `Sources/LaboLaboStore/AppDataDirectory.swift` | `store/data_dir.rs` |

This wave ports session/appState SQLite persistence, via `rusqlite`
(`bundled` feature — SQLite is compiled in, no system library dependency)
instead of GRDB. It lives as a `store` module inside `labolabo-core` per the
porting brief, not a new crate.

### GRDB on-disk compatibility

An existing user's `~/Library/Application Support/LaboLabo/labolabo.db` was
created and is still read/written by Swift's GRDB `DatabaseMigrator`, which
tracks applied migrations in a `grdb_migrations(identifier TEXT NOT NULL
PRIMARY KEY)` table. This port never reads or writes `grdb_migrations` — it
stays exclusively under the Swift side's management, verified in
`tests/store_golden.rs`'s `never_touches_grdb_migrations_across_a_full_read_write_delete_cycle`
test (byte-identical `grdb_migrations` contents before/after a full
read+write+delete cycle through the Rust port).

Instead of its own migration ledger, `store::database::SessionDatabase::ensure_schema`
reconciles the `session`/`appState` tables to the v3 shape (the final state
of Swift's three migrations: `v1`, `v2-agentSession`, `v3-adapter`) via
idempotent, existence-checked DDL: it creates each table outright with its
full v3 definition if the table doesn't exist yet (a brand-new database),
or — if it already exists, at *any* prior GRDB migration level (v1 through
v3) — adds only whatever columns `PRAGMA table_info` shows are missing. One
code path handles a fresh database, a v1-only database, a v2 database, and
an already-v3 database (a no-op) uniformly. Column types/constraints are
copied from `SessionDatabase.swift`'s migrator verbatim, cross-checked
against GRDB's own `TableDefinition.primaryKey`/`column` source (a
non-`.integer` `primaryKey(_:_:)` column gets an explicit `NOT NULL`, which
GRDB itself adds to route around a SQLite quirk — see
`TableDefinition.swift`'s doc comment).

### `Date` columns — the trickiest part of this port

GRDB's `Date: DatabaseValueConvertible` extension always *writes*
`"yyyy-MM-dd HH:mm:ss.SSS"` in UTC (fixed `DateFormatter`, always 3
fractional digits, never a zone suffix) but is considerably more lenient
when *reading*: it accepts `YYYY-MM-DD[ T]HH:MM[:SS[.SSS]]` with an optional
trailing `Z`/`+HH:MM`/`-HH:MM`, or — if the column's SQLite storage class is
numeric rather than TEXT — falls back to interpreting the value as
`timeIntervalSince1970` **seconds**. `store::datetime` (`format_grdb_datetime`
/ `parse_grdb_datetime`) reimplements both directions; `store::database`
handles the numeric-storage-class fallback directly against
`rusqlite::types::ValueRef` (see `store::datetime`'s module doc comment for
the full contract, cross-checked line-by-line against GRDB's
`Date`/`DatabaseDateComponents`/`SQLiteDateParser` sources, and for why the
parser's "greedy, single trailing check" structure is a deliberate
restructuring of `SQLiteDateParser`'s "strict incremental" one that is
behaviorally equivalent for every input).

One genuine, documented divergence: out-of-range calendar components (e.g.
month `13`) are rejected (`chrono::NaiveDate::from_ymd_opt` returns `None`),
where Swift's `Calendar(identifier: .gregorian).date(from:)` would instead
*roll over* into a different, valid date. Every real `addedAt` value in
production was itself written by the format-side counterpart of this same
parser, so it's always in-range; this only matters for hand-edited/corrupted
data, and rejecting outright is the safer failure mode for a persistence
layer than silently reinterpreting a corrupt date as a different one.

`SessionRecord.added_at` is `chrono::DateTime<Utc>` rather than a bespoke
type — GRDB's storage format never needs better than millisecond precision,
which `DateTime<Utc>` carries faithfully without forcing a timezone-naive
representation on future callers.

### Other faithfully-carried-over quirks

- **`upsert`** is one `INSERT ... ON CONFLICT(id) DO UPDATE` statement, not
  a literal transliteration of `record.save(db)` — GRDB's
  `PersistableRecord.save` is documented as "update if a matching primary
  key row exists, insert otherwise," which is exactly what the `ON
  CONFLICT` clause expresses in a single round-trip.
- **`app_state_entries`'s NULL-drop**: the Swift source binds each row with
  `if let key: String = row["key"], let value: String = row["value"]` —
  conditional binding through `Optional<String>: DatabaseValueConvertible`.
  A row whose `value` is SQL NULL fails that binding and is **silently
  dropped from the result** (not included with an empty string). The Rust
  port reproduces this by skipping `NULL`-valued rows rather than mapping
  them to `""`. See the `null_value_row_is_dropped`/
  `app_state_entries_drops_the_real_grdb_written_null_row` tests (the latter
  against a row a real GRDB run actually wrote as NULL, not a hand-authored
  one).
- **`app_state`/`selected_session_id` NULL-collapsing**: GRDB's
  `fetchOne` is documented to return `nil` both when the query returns no
  row *and* when the first row's value is NULL — so a key that exists with
  an explicit NULL value reads back identically to a key that was never
  set. The Rust port matches this (see
  `app_state_on_a_real_null_valued_key_is_none_not_the_key_missing`).

### Golden fixture (`fixtures/store/fixture.db`)

Unlike waves 1/2 (canonical-JSON comparison against a Swift-produced
`fixtures/expected/**` file) or wave 3 (its own JSON-based oracle), this
wave's oracle output *is* a SQLite database file: `fixtures/store/fixture.db`
was produced by a disposable Swift package (not checked into this repo, per
the "leave no trace" convention `fixtures/generate.swift` and the wave 3
tiling driver already established) that depends on this repo's real
`LaboLaboStore` product via a local SwiftPM path dependency and drives the
real `SessionDatabase.init(url:)` / `.upsert` / `.setSelectedSessionID` /
`.setAppState` — the exact code path a running LaboLabo app uses. See
`tests/store_golden.rs`'s module doc comment for the exact regeneration
recipe and for the fixture's full contents (4 session rows — one fully
populated with Japanese text, one with every optional column NULL, one with
quotes/backslash/newline/tab/emoji content to exercise parameter binding
rather than string interpolation, one with an exact-midnight `addedAt` —
plus `appState` rows covering `selectedSession`, a prefix-filtered group
with one NULL-valued and one empty-string-valued entry, and an
outside-the-prefix key).

`cargo test` opens a fresh copy of the fixture (never mutating the checked-in
file) and asserts: `all_sessions`/`selected_session_id`/`app_state`/
`app_state_entries` match hand-verified expected values (transcribed from a
`sqlite3 fixture.db` dump, documented in the test file); a subsequent
`upsert` writes a raw `addedAt` TEXT value that is byte-identical to what
GRDB's own `DateFormatter` would produce; and `grdb_migrations` is
byte-identical before and after a full read+write+delete cycle. All 8
`SessionPersisting` operations are additionally exercised through the trait
object (not just the inherent methods) in
`all_8_operations_are_reachable_through_the_session_persisting_trait`.

## Wave 5b-3 (Task model: `store::task_record` / `store::task_database` / `branch_naming`)

Unlike every wave above, this one has **no Swift source**: the Task model
(`plans/012-task-model-and-control-cli.md` §1 — "1 作業 = 1 worktree (or
attached directory) = 1 tile/tab tree", decided 2026-07-14 to ship only in
the Rust port) is new product surface. The `labolabo-core` pieces:

- `store/task_record.rs` — `Task` (`id`: UUID v4, `repo_key`/`repo_root`/
  `repo_name` from `GitEngine::repo_info`, `kind: Worktree { branch, base,
  path } | Attached { directory }`, `title`, `layout: TileLayout`, `status:
  active|done|archived`, `created_at`/`last_active_at`, `sort_order`, and a
  reserved `agent_bindings` column for the plan's per-tab agent bindings).
- `store/task_database.rs` — `TaskDatabase` (rusqlite): CRUD + selected-task
  app-state, with its own ordered-migration ledger (`schemaMigrations`
  table). **No GRDB compatibility constraint**, and deliberately a separate
  database *file* from the Swift app's: `TaskDatabase::default_path()` is
  `<data dir>/LaboLabo/tasks.db` (`store::rust_app_data_dir`; before the
  1.1.0 rename this was a separate `LaboLabo-rs/` directory — a one-time
  startup migration, `store::migrate_legacy_rust_data_dir`, moves an old
  `tasks.db` across), never the Swift `LaboLabo/labolabo.db` — two live
  apps must never write the same SQLite *file* (sharing the directory with
  different filenames is fine), and this schema shares nothing with the
  GRDB one (which stays untouched for Swift-data import). A Task's
  `layout` column stores `TileLayout::to_json` verbatim, so the tile
  tree's existing byte-compatibility contract carries over unchanged.
- `branch_naming.rs` — pure `generate_branch_name(prefix, date, existing)`
  (`labolabo/<YYYYMMDD>-<n>`, collision-skipping) for the "new worktree
  Task" flow; kept in core (not `labolabo-app`) so the future control CLI
  (plan §2) can share it.

No golden coverage (nothing to compare against — there is no Swift
implementation); unit tests cover CRUD round-trips (including `TileLayout`
through the DB), migration-ledger idempotence, on-disk reopen persistence,
and malformed-value error surfacing. The UI driving all of this lives in
`crates/labolabo-app` (see its README's "The Task model" section).

## Wave 6a (macOS `.app` bundle)

`scripts/bundle-macos.sh` packages the three built binaries
(`labolabo-app`, the gpui GUI; `labolabo`, the control CLI; `labolabo-hook`,
the Claude Code hooks forwarder — see "Wave 4b" above) into a distributable
`LaboLabo.app` (named `LaboLabo-rs.app` before the 1.1.0 rename — see
"1.1.0 の正式名改名" under the RC リリース手順 section below), mirroring
the Swift app's own release packaging
(`.github/workflows/release-build.yml`):

```sh
rust/scripts/bundle-macos.sh
# -> rust/target/bundle/LaboLabo.app
# -> rust/target/bundle/LaboLabo-<version>.zip
```

It runs `cargo build --release -p labolabo-app -p labolabo-core`, then
assembles `Contents/MacOS/{labolabo-app,labolabo,labolabo-hook}`,
`Contents/Resources/AppIcon.icns`, and `Contents/Info.plist`, ad-hoc signs
(`codesign --sign -`, the same signing identity the Swift app's release
build uses — no Developer ID / notarization), and zips with `ditto`.

A few design decisions worth calling out:

- **All three binaries live side by side in `Contents/MacOS/`.** This isn't
  just a packaging convenience: `crates/labolabo-app/src/hooks.rs`'s
  `resolve_hook_binary` finds `labolabo-hook` as the sibling of
  `std::env::current_exe()`, so this layout is what makes hooks injection
  (agent status dots, session memory, resume-at-restore) work inside the
  bundle at all — no code change was needed, the existing sibling-directory
  resolution already fits an app bundle's flat `MacOS/` directory.
- **Bundle identifier**: `com.love-rox.labolabo` as of the 1.1.0 rename —
  the Swift app's own bundle ID, inherited deliberately now that the Swift
  app is retired (before 1.1.0 this was `com.love-rox.labolabo-rs`, an
  `-rs` suffix chosen so the two then-coexisting apps never collided; the
  on-disk data directory made the same move, `LaboLabo-rs` → `LaboLabo` —
  see "Wave 5b-3" above for the migration).
- **Version**: `CFBundleShortVersionString` is *not* the workspace crates'
  own `Cargo.toml` `version` (still `0.1.0` — this port is pre-1.0
  internally) — per explicit product direction, this bundle is versioned as
  a major bump from the Swift app's release line
  (`Config/Version.xcconfig`'s `MARKETING_VERSION`), not a continuation of
  either the Swift 0.x line or the crates' own 0.1.0. As of the RC release
  wave (see "RC リリース手順" below) this is single-sourced from
  `rust/VERSION` (one plain-text line, e.g. `1.0.0-rc.1`), with a
  `LABOLABO_RS_VERSION` env-var override that `bundle-macos.sh` also
  forwards into the `cargo build` it runs — so the packaged zip's file name
  *and* the compiled binary's own About-panel version (`crates/labolabo-app/
  src/menus.rs` `APP_VERSION`, injected by `build.rs`) always agree, with no
  manual sync step. `CFBundleVersion` (the build number) follows the Swift
  app's own convention: `git rev-list --count HEAD`.
- **Icon**: reuses the Swift app's own artwork
  (`app/Sources/Assets.xcassets/AppIcon.appiconset/*.png`) rather than
  shipping unbranded or placeholder icons — those PNGs already use
  `iconutil`'s exact `.iconset` naming convention, so the script copies
  them into a scratch `.iconset` directory and converts with
  `iconutil -c icns` directly.
- **`LSMinimumSystemVersion`**: `10.15.7`, gpui's own
  Metal-backed-renderer floor (its `build.rs` passes this as the macOS
  linker version-min), not the Swift app's unrelated `14.0` deployment
  target.

`.github/workflows/rust-app-bundle.yml` runs this script on `macos-15` and
uploads the resulting `.zip` as a workflow artifact. It's
**`workflow_dispatch`-only** (no push/PR/release trigger) — the Rust port
isn't part of the release-please/`release-build.yml` release flow yet;
that integration is a separate future decision.

Wave 7a added the Linux counterpart: `scripts/package-linux.sh` packages
the same three binaries into a portable
`LaboLabo-linux-<version>-<arch>.tar.gz` (flat `bin/` + freedesktop.org
`.desktop` launcher + per-user `install.sh` + PNG icon reused from the
Swift app's artwork + README), and `rust-app-bundle.yml`'s `package-linux`
job runs it on `ubuntu-latest` under the same `workflow_dispatch`-only
policy. See `crates/labolabo-app/README.md`'s "Linux (wave 7a)" section for
system dependencies and the verification caveats (built + headless-tested
in CI; real-desktop GUI launch unverified).

## Windows core wave (Named Pipe transports / tool locator / process kill)

Implements the three Windows gaps this crate had carried as reserved
chapters and cfg'd stubs, making `labolabo-core` (and the `labolabo` CLI
bin in `labolabo-app`) fully functional on Windows — the groundwork for the
app (gpui) Windows wave, which is separate future work.

- **hooks Named Pipe transport** (`hooks::NamedPipeEventTransport`,
  `#[cfg(windows)]`): docs/hooks-protocol.md §4.2, promoted from the v1
  "Windows 代替（未実装）" bullet to a real spec by this wave. Byte-mode,
  inbound-only pipe named `\\.\pipe\labolabo-<10hex>`
  (`hook_settings::hook_pipe_name_from_uuid` — pure, compiled everywhere);
  same "1 connection = 1 event, read to EOF" contract as AF_UNIX (the
  client's close is thunked to EOF). `forward_hook` and the
  `labolabo-hook` bin now forward on Windows too (`any(unix, windows)`).
- **control Named Pipe transport** (`control::ControlServer` /
  `send_control_request`, `#[cfg(windows)]`, same signatures as unix):
  docs/control-protocol.md §9. Duplex **message-mode** pipe named
  `\\.\pipe\labolabo-control-<10hex>`
  (`control_protocol::control_pipe_name_from_uuid`) — Named Pipes have no
  half-close, so the OS-preserved message boundary replaces the unix
  "write then `shutdown(SHUT_WR)`" framing (1 connection = 1 request
  message = 1 response message; same JSON, same exit codes). This makes
  the `labolabo` control CLI build and run on Windows unchanged.
- **Same-user ACL** (`windows_pipe_security`, crate-internal): both pipe
  servers create their pipe with a protected DACL granting access only to
  the current user's SID and SYSTEM — the Windows counterpart of
  `chmod 0600` — and fail closed (refuse to bind) if it can't be built.
- **`ToolLocator` on Windows**: the former `unimplemented!()` arm is now a
  PATHEXT-aware `PATH` scan (no `where` subprocess — the search rule is
  simple enough that shelling out buys nothing; no fixed candidates or
  login-shell fallback either, see the module doc comment).
- **`process` kill escalation on Windows**: `run_with_timeout`'s
  terminate/kill pair now maps to `taskkill /PID` → `taskkill /F /PID`
  (previously no-ops, which made the timeout path hang until the child
  exited on its own), with `cmd /C` counterparts of the unix process
  tests.

Windows dependencies (all `[target.'cfg(windows)'.dependencies]`, none on
unix builds): `interprocess` (sync Named Pipe layer; default features — no
tokio/async, this crate stays runtime-free), `recvmsg` (message-receive
buffer types the control transport's framing needs), `widestring` (SDDL
string conversion), `windows-sys` (current-user SID lookup for the DACL).
See `crates/labolabo-core/Cargo.toml` for the full selection rationale,
including why `interprocess` was chosen over hand-rolled `windows-sys`
transport code.

Tests run for real on the `rust (windows-latest)` CI job: the 6 bus
round-trip tests and 5 control round-trip tests (shared bodies with the
unix runs, per-OS transport underneath), a Named Pipe end-to-end test of
the compiled `labolabo-hook` binary, `cmd`-based `ToolLocator` and
process-runner tests, and the `windows_pipe_security` SDDL tests. Local
verification from macOS: `cargo check/clippy/build --target
x86_64-pc-windows-gnu` (mingw-w64), including a full link of the
`labolabo` CLI bin.

## RC リリース手順（RC release wave）

### 1.1.0 の正式名改名（LaboLabo-rs → LaboLabo）

Swift（macOS ネイティブ）版の引退決定に伴い、1.1.0 から Rust 版が正式名
**LaboLabo** を引き継いだ。対応表:

- 配布物: `LaboLabo.app` / `LaboLabo-<version>.zip`（macOS）、
  `LaboLabo-linux-<version>-<arch>.tar.gz`、
  `LaboLabo-windows-<version>-<arch>.zip`。リリースタイトルも
  `LaboLabo <version>`。
- Bundle ID: `com.love-rox.labolabo`（Swift 版の ID を継承）。
- データディレクトリ: `<data dir>/LaboLabo/tasks.db`（旧
  `LaboLabo-rs/tasks.db` は初回起動時に自動で rename 移動 —
  `store::migrate_legacy_rust_data_dir`。失敗時は旧パスをそのまま使い
  続けるフォールバックあり）。
- **変えないもの**: タグ体系は `rs-v*` のまま（Swift 版の `v*` タグと
  過去分も含めて分離し続けるため）。3 実行ファイル名
  （`labolabo-app`/`labolabo`/`labolabo-hook`）も不変（hooks の隣接
  バイナリ解決を壊さない）。環境変数 `LABOLABO_RS_DATA_DIR` /
  `LABOLABO_RS_VERSION` / `LABOLABO_RS_INSTALL_DIR` も互換のため
  旧名のまま。
- Homebrew: `rs-v*` リリースの publish 時に `.github/workflows/
  rust-cask-bump.yml` が tap の **`Casks/labolabo.rb`** を bump する
  （labolabo cask が Rust 版の正式 cask。旧 labolabo-rs cask の廃止は
  tap 側の作業）。

`.github/workflows/rust-release.yml` は、Rust 版 labolabo-app を Mac/
Linux/Windows 3 アーティファクト付きの GitHub **pre-release**（draft）
として発行するための配管。**この workflow 自体は pre-release の実発行・
タグ付けは行わない** — `--draft` フラグにより、GitHub は draft のままでは
タグを実際にリポジトリへは打たない（人間が Releases 画面で "Publish
release" するまでタグは作られない）。手順は次の通り:

1. **`workflow_dispatch` の実行** -- GitHub の Actions タブから
   "Rust release (RC)" を選び、`tag` 入力に `rs-v` プレフィクス付きの
   タグ（例 `rs-v1.0.0-rc.1`）を指定して実行する。既存の release-please
   管理下の Swift 版タグ（`v*`）と衝突しないよう、この `rs-v*` プレフィクス
   は必須（`resolve-version` ジョブが検証・拒否する）。タグから `rs-v` を
   剥がした残りがそのままバージョン文字列（`1.0.0-rc.1`）になり、3 プラット
   フォームのビルド・パッケージング（`bundle-macos.sh`/`package-linux.sh`/
   `package-windows.ps1`、いずれも `LABOLABO_RS_VERSION` env 経由でこの
   ワークフローが明示的に渡す）と `crates/labolabo-app/src/menus.rs`
   `APP_VERSION`（`build.rs` 経由でコンパイル時に注入）の両方に一致する
   — リポジトリの `rust/VERSION` ファイルを都度書き換える必要はない。
2. **draft release の確認** -- 3 ジョブ（`bundle-macos`/`package-linux`/
   `package-windows`）が green になった後、`create-release` ジョブが
   3 アーティファクトを集約して `gh release create --prerelease --draft`
   する。GitHub の Releases 画面で draft を開き、3 アーティファクトが
   揃っていること・リリースノート（`rust/RELEASE_NOTES_TEMPLATE.md` を
   バージョン/タグで埋めたもの）の内容を確認する。
3. **publish** -- 内容に問題なければ、GitHub の Releases 画面で "Publish
   release" を押す。この操作で初めてタグ（`rs-v1.0.0-rc.1` 等）が実際に
   リポジトリへ作られ、pre-release が公開される。
4. **サイト（labolabo-site PR #1）マージ** -- publish 後、ダウンロード
   リンクを最新の RC に向けるサイト側の変更（labolabo-site リポジトリの
   該当 PR）をマージする。

`rust-app-bundle.yml`（既存の Rust 手動ビルド、`workflow_dispatch` のみ・
アーティファクトの workflow 出力止まりでリリース化はしない）とは独立した
別ファイルのまま運用する — 統合は将来判断（過剰な工事はしないという方針）。

### アップデート確認（Rust 版、`crate::update_check`）

Rust 版アプリは起動時に一度だけ、バックグラウンドで GitHub Releases を
確認し、新しいバージョンが見つかればサイドバーに控えめなバナーを表示する
（`crates/labolabo-app/src/update_check.rs`、Swift 版 `UpdateChecker` の
軽量移植 — 常駐ポーリングはせず、OS 通知も出さない）。HTTP は新規依存を
増やさず `curl -fsSL --max-time 5` を子プロセスとして呼ぶだけで、`curl`
不在やネットワーク失敗は静かに無視する（UI には一切出ない）。RC ビルド
（バージョン文字列に `-rc` を含む）は `rs-v*` タグの最新（pre-release 込み、
`/releases?per_page=10` をフィルタ）、安定版ビルドは `/releases/latest`
（`rs-v*` でなければ Swift 版のタグとみなして無視）を見る。バナーの「×」
（閉じる）操作は「今後このバージョンを通知しない」を兼ねる（appState の
`ignoredUpdateVersion` へ永続化）。設定画面の「アップデートを自動確認」
トグル（既定 on）と、スモークテスト/CI 向けの `LABOLABO_NO_UPDATE_CHECK=1`
環境変数の両方で独立に無効化できる。

## Wave 12（ghostty-vt を配布物の既定バックエンドに）

### 配布 vs 開発の既定バックエンド

VT コアは最初から選択式（`backend-alacritty`/`backend-ghostty-vt`、
`crates/labolabo-term/Cargo.toml`）だったが、これまで**開発・配布の両方**
で既定は alacritty のままだった。プロジェクトの大前提は「VT コアは
libghostty-vt が本命」（Ghostty アイデンティティそのもの）なので、この波
から二層に分ける:

- **開発既定は alacritty のまま（変更なし）**: `cargo build`/`cargo test`
  や CI の `rust`/`rust-app*` ジョブは今まで通り crates.io だけで完結する。
  Zig トールチェインを要求しない、という制約自体は変えない（Windows 開発
  の都合・`cargo test` の敷居を上げない判断 —
  `crates/labolabo-term/Cargo.toml`/`crates/labolabo-term/README.md`
  「Backends」節参照）。
- **配布既定は ghostty-vt**: `rust/scripts/bundle-macos.sh`（macOS）と
  `package-linux.sh`（Linux）はこの波から既定でこちらを選ぶ。ツールチェイン
  （Zig 0.16 + `GHOSTTY_SOURCE_DIR`）が見つからない場合はセットアップ手順
  つきのエラーで即停止する（`cargo`のビルドエラーへ丸投げしない）。
  `LABOLABO_VT_BACKEND=alacritty` 環境変数で、従来どおりの alacritty ビルド
  へ明示的に戻せる（緊急ハッチ）。
  ```sh
  ./scripts/bundle-macos.sh          # 既定: ghostty-vt（toolchain 必須）
  LABOLABO_VT_BACKEND=alacritty ./scripts/bundle-macos.sh   # 従来どおり alacritty
  ```
- **Windows は alacritty のまま**: libghostty の Windows ビルドは未検証
  （この開発ラインに Windows 実機がない）ため、`scripts/package-windows.ps1`
  にバックエンド切替は入れていない（`cargo build`の既定 = alacritty のまま
  呼ぶだけ）。
- **CI**: `.github/workflows/rust-release.yml`（`bundle-macos`/
  `package-linux` ジョブ）と `rust-app-bundle.yml`（`bundle`/
  `package-linux` ジョブ）の両方に、既存の `.github/workflows/ci.yml`
  `rust-term-ghostty` ジョブと**全く同じ**ツールチェイン段取り（vancluever/
  ghostty の固定 SHA チェックアウト + `mlugg/setup-zig@v2` で Zig 0.16）を
  追加した。同じ `GHOSTTY_REF` が3ファイルに重複しているので、fork の
  ピンを更新する際は3つとも揃えること。`rust-term-ghostty` 自体は
  `continue-on-error: true`（実験的な継続実証用）のままだが、配布ジョブの
  ほうは通常どおり失敗すればワークフロー全体が失敗する（ghostty-vt の
  ビルド失敗を握りつぶさない）。

ローカルでの ghostty-vt ビルド検証は `crates/labolabo-term/README.md`の
「Building the ghostty-vt backend」節と同じ前提（Zig 0.16 + Zig-0.16
対応 Ghostty フォークの `GHOSTTY_SOURCE_DIR`）:

```sh
export GHOSTTY_SOURCE_DIR=/path/to/vancluever-ghostty-checkout   # zig-0.16 ブランチ。pin SHA は ci.yml の rust-term-ghostty ジョブ参照
export PATH="/path/to/zig-0.16.0/bin:$PATH"
cd rust
cargo test -p labolabo-term --no-default-features --features backend-ghostty-vt
./scripts/bundle-macos.sh   # macOS のみ -- .app バンドルも ghostty-vt でビルドされる
```

### About 表記

`labolabo-app` の About オーバーレイ（`crates/labolabo-app/src/menus.rs`
`render_about_overlay`）は、ビルド時の feature に応じて「VT: libghostty-vt」
または「VT: alacritty」を表示する（`VT_BACKEND` 定数 — `labolabo_term::
ACTIVE_BACKEND_NAME` の再エクスポートで、cfg 判定を crate 境界をまたいで
二重実装しない設計）。サポート対応時にどちらのビルドが動いているか
判別できる。

## Wave 14（`rust-term-ghostty` の断続的 SIGILL を根本修正）

`rust-term-ghostty (ubuntu-latest)` の `cargo test`（`backend_common`）が
断続的に `signal: 4, SIGILL: illegal instruction` でプロセスごと落ちる問題
を調査し、根本原因を特定して修正した。

### 原因

`libghostty-vt-sys`（`libghostty-vt` の -sys クレート）の `build.rs` は、
クロスコンパイル時（`TARGET != HOST`）だけ `zig build` に `-Dtarget=<triple>`
を渡し、ネイティブビルド（`TARGET == HOST`、ubuntu-latest の
`cargo test` はこれに該当）では省略して Zig に自動検出させていた。
Ghostty 本体の `build.zig`/`Config.zig` は `b.standardTargetOptions(.{})`
を素通しするだけで、macOS 向けだけ `genericMacOSTarget()` で CPU を
`generic` に強制する回避策（github.com/mitchellh/ghostty/issues/1640）を
持つが、Linux 向けには同等の処理が無い。結果として:

- **`-Dtarget` 省略（ネイティブ）**: Zig がビルドマシンの CPU を実機検出し、
  そのマシンが持つ拡張命令（AVX-512 等）をすべて焼き込む
  （`-femit-llvm-ir` の `target-features` 属性で実測確認済み）。
- **`-Dtarget=x86_64-linux-gnu` のように明示（cpu サフィックス無し）**:
  そのアーキテクチャの可搬な baseline CPU（SSE/SSE2 相当のみ）に解決される。

GitHub Actions の `ubuntu-latest` はランナー世代が実行ごとに異なる
（不均質な fleet）うえ、`mlugg/setup-zig@v2` の既定キャッシュが
`.zig-cache` を**別の・無関係な過去の run**からも `restore-keys` の
prefix フォールバックで復元する（実際の CI ログで、別 run が保存した
キャッシュを復元しているのを確認済み）。そのため「幅広い CPU 機能を持つ
ランナーがビルドした `libghostty-vt.a`」が「その機能を持たない別ランナー」
に復元・リンクされ、対応していない命令に到達した瞬間に SIGILL で落ちる
— これが観測されていた断続的な失敗の実体。直近 15 run のサンプルでは、
`.zig-cache` が空だった（コールドキャッシュ = 同一ランナー上で自己完結
ビルド）3 run は全て成功、復元ありの run は成否が混在しており、CPU 世代
不一致仮説と整合していた。もう一つの仮説（Zig `ReleaseFast` での
`unreachable` がトラップ命令化して SIGILL になる、libghostty-vt 自体の
バグ）は、ローカルでのフレッシュビルド + 60 回リピート実行が全成功した
ことと、コールドキャッシュ run が全成功だったことから積極的な支持が
得られず、採用していない（実機の x86_64 Linux での大規模リピートは
未実施 — 完全には棄却できないが、観測データは仮説 1 で一貫して説明できる）。

### 修正

`rust/vendor/libghostty-vt-sys/`（`[patch.crates-io]` で `rust/Cargo.toml`
から差し替え）に、公開クレート `libghostty-vt-sys` 0.2.0 のローカル
パッチ版を用意した。差分は `build.rs` の一箇所のみ:
**`-Dtarget` を常に渡す**（`TARGET != HOST` の条件分岐を撤廃）。
これによりネイティブビルドも含めて常に baseline CPU へ解決されるため、
CI のキャッシュ復元やランナー世代の違いに関係なく安全になる。
詳細な経緯は `rust/vendor/libghostty-vt-sys/README.md` と `build.rs` の
"LOCAL PATCH" コメント参照。upstream（github.com/uzaaft/libghostty-rs）に
同等の修正が入り次第、このベンダコピーは削除する。

**配布物への影響**: `bundle-macos.sh`/`package-linux.sh` はどちらも
同じ `libghostty-vt-sys` を使うため、この修正で Linux/macOS の配布
バイナリも baseline CPU でビルドされるようになり、「リリースランナーの
たまたまの CPU 世代がユーザーの CPU で SIGILL する」というリスクが
解消される。macOS は元々 `genericMacOSTarget()` により安全だった
（world 世代差の影響は小さいと見ているが、Apple Silicon 世代間の
命令セット差は x86_64 の SSE→AVX-512 ほど大きくないとはいえゼロではない
ため、今回の修正で明示的にも baseline 化された点は保険として有効）。

## Wave 15（Kitty keyboard protocol: `claude` の Shift+Enter が改行にならない）

実機バグ報告「ターミナルでの `claude` における Shift+Enter がうまく動いて
いない」に対応した。Claude Code の TUI は Shift+Enter で改行を挿入するが、
これは端末が [Kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)
の「disambiguate escape codes」フラグ（`CSI > 1 u` で push）をサポートして
初めて機能する。従来の `keys.rs` は `"enter"` を修飾キー無視で常に `\r` に
していたため、Shift+Enter が素の Enter と区別できず、常に「送信」扱いに
なっていた。

### バックエンド照会の設計

`VtBackend` に `kitty_disambiguate(&self) -> bool` を追加した
（`bracketed_paste`/`mouse_mode` と同じ「毎バイトバッチ後に問い合わせて
`Arc<AtomicBool>` へ publish、呼び出し側は non-blocking で読む」パターン。
`TermSession::kitty_disambiguate()`）。ブール一つに絞ったのは、
`labolabo-app` 側が必要とするのが「disambiguate が有効か」だけだからで、
Kitty protocol の他の progressive-enhancement ビット（event types /
alternate keys / all-keys-as-escape / associated text）は今回のスコープ
外（このクレートはそもそも key-*down* しか送らないため、event-types 系は
フックする先すら無い）。

両バックエンドの実装は非対称:

- **`libghostty-vt`（本番バックエンド）**: Kitty keyboard protocol の
  モードスタックを**無条件に**追跡している（オプトイン設定なし）。
  `Terminal::kitty_keyboard_flags()` が FFI 越しにそのまま使え、実装は
  1メソッドの直接呼び出しで完結する。裏付け: ベンダコピー
  `rust/vendor/libghostty-vt-sys/src/bindings.rs` に
  `Data::KITTY_KEYBOARD_FLAGS = 8` が既に存在し、Ghostty 本体の Zig ソース
  （`~/ghq/.../labolabo-spikes/ghostty-zig016-src/src/terminal/c/terminal.zig`）
  にはこの getter 専用の Zig テスト（`test "get kitty_keyboard_flags"`、
  `CSI > 3 u` を push して `flags == 3` を確認）まである。push/pop/set/query
  の parse・状態更新（`stream_terminal.zig`）も常時有効で、クエリ応答
  （`CSI ? u` -> `CSI ? <flags> u`）は既存の `on_pty_write` コールバック
  経由でそのまま PTY に返る。
- **`alacritty_terminal` 0.26（CI 既定バックエンド）**: `Term`
  の Kitty-keyboard 系 `Handler` メソッド（`push_keyboard_mode`/
  `pop_keyboard_modes`/`set_keyboard_mode`/`report_keyboard_mode`、
  `term/mod.rs`）は **`Config::kitty_keyboard` が `true` のときしか本体を
  実行しない**（`false` が既定値）。`vte::ansi::Processor` のディスパッチ
  自体は `CSI > u`/`CSI < u`/`CSI = u`/`CSI ? u` を無条件にパースするため、
  「シーケンスは読めるが状態には一切反映されない」まま黙って捨てられる
  挙動になっていた。`AlacrittyBackend::new` で `Config { kitty_keyboard:
  true, .. }` を明示することで、この既定を上書きして push/pop/query が
  実際に効くようにした（`TermMode::DISAMBIGUATE_ESC_CODES` を素直に
  `mode().contains(..)` で読める）。**両バックエンドとも `CSI u` の
  parse・push/pop 自体はサポート済みで、共通レイヤでのシーケンス検出に
  頼る必要はなかった**（ブリーフで挙げていた代替方針は不要と判断）。

共有ヘッドレステスト `tests/backend_common.rs::
kitty_disambiguate_reflects_csi_push_pop`（`printf '\033[>1u'` で push ->
`true`、`printf '\033[<u'` で pop -> `false`、他のトグル系テストと同じ
「子を `read` でブロックさせてから次の書き込みを送る」フレーク対策込み）
を両バックエンドに対して実行し、green を確認済み（下記ゲート結果）。

### エンコード対応表（`labolabo-app::keys::keystroke_to_bytes`）

Kitty protocol の仕様どおり、無修飾の Enter/Tab は disambiguate 有効時も
legacy バイトのまま（仕様の明記どおり: crash 後の端末で `reset<Enter>` が
打てるようにするための意図的な例外）。**modifier 付きの Enter/Tab のみ**
`CSI <code>;<modifier> u` に再エンコードする（`modifier = 1 +
shift(1)/alt(2)/ctrl(4)` の和）:

| キー | disambiguate 無効時（従来どおり） | disambiguate 有効時 |
|---|---|---|
| Enter（無修飾） | `\r` | `\r`（仕様の例外、変化なし） |
| Shift+Enter | `\r`（区別不能だった = 今回のバグ） | `\x1b[13;2u` |
| Alt+Enter | `\r` | `\x1b[13;3u` |
| Ctrl+Enter | `\r` | `\x1b[13;5u` |
| Ctrl+Alt+Shift+Enter | `\r` | `\x1b[13;8u` |
| Tab（無修飾） | `\t` | `\t`（仕様の例外、変化なし） |
| Shift+Tab | `\t` | `\x1b[9;2u` |
| Backspace / Escape / 矢印 / Ctrl-<letter> | 変化なし | 変化なし（今回のスコープ外 -- 下記） |
| Cmd/Super 修飾 | どのキーも常に `None`（アプリショートカット用） | 同左（disambiguate 判定より前に return） |

Backspace/Escape の modifier 付きエンコードは意図的にスコープ外とした:
現状このアプリはどちらにも modifier 付きバインドを持たず曖昧さが無い上、
Backspace は仕様上の例外規定がやや曖昧（"Enter, Tab and Backspace は
legacy のまま" と読める一文があり、modifier 付きでも変わらないのか
`CSI 127;<mod> u` になるのか、要約ベースの一次情報だけでは断定できな
かった）ため、確度の低い実装を足すより明確に「今回のスコープ外」として
残す判断をした。

### フォールバック（実装しない判断）

Kitty protocol 非対応の端末バックエンド設定（`kitty_disambiguate() ==
false` のまま）では、Shift+Enter は従来どおり素の Enter と同じ `\r` に
なる -- Claude Code 側には `\` + Enter による手動改行が常に効くフォール
バックがあるため、このアプリ側で追加のフォールバック実装（例:
`\x1b\r` のような alternate encoding を独自に送る等）はしなかった。
`/terminal-setup` 相当の追加設定も不要 -- 両バックエンドとも Kitty
protocol の push/pop を無条件にパースする（前述）ので、Claude Code が
自分で `CSI > 1 u` を送った時点で自動的に有効になる。

### 品質ゲート結果

- `cargo build/test/clippy(-D warnings)` を `-p labolabo-term`
  （**alacritty backend（既定）と ghostty-vt backend（`PATH` に
  `~/.local/opt/zig-aarch64-macos-0.16.0` を先頭追加 +
  `GHOSTTY_SOURCE_DIR=~/ghq/.../labolabo-spikes/ghostty-zig016-src` で
  ローカル実ビルド）の両方**）/ `-p labolabo-app` / ルート
  `default-members`（`labolabo-core` + `labolabo-term`）それぞれで実行、
  全 green（`labolabo-term`: unit 6 + 統合 26、うち新規
  `kitty_disambiguate_reflects_csi_push_pop` 込み。`labolabo-app`:
  385 lib tests、うち `keys::` 17 -- 既存 9 + 新規 8）。
- `cargo fmt --check`（ワークスペース全体）green。
- スモーク: `LABOLABO_RS_DATA_DIR=$(mktemp -d)` の上で `cargo run -p
  labolabo-app` を約5秒起動して kill -- パニック/クラッシュ出力なし
  （このマシンに `timeout(1)` が無かったため、バックグラウンド起動 +
  `sleep 5` + `kill` で代替。合成入力・spawn直後のデフォルト状態 assert
  なし）。

### 未検証

- **実際の `claude` での Shift+Enter 改行はユーザー未確認。** 上記は
  「バックエンドが `CSI > 1 u` を push したら `kitty_disambiguate()` が
  `true` になり、`keys::keystroke_to_bytes` が `\x1b[13;2u` を書き込む」
  というヘッドレス統合テスト・ユニットテストで固めた設計上の裏付けで
  あり、実機の LaboLabo アプリ上で `claude` を起動し、実際に Shift+Enter
  キーを叩いて改行が入ることまでは確認していない（合成キーボード入力は
  この移植の検証方針で禁止されているため）。
- **`libghostty-vt` 経由の実クエリ応答（`CSI ? u` への `CSI ? <flags> u`
  返信）は Ghostty 側の既存機構をそのまま経由するだけで新規コードでは
  ないため個別検証していない**が、`title`/`bracketed_paste` など既存の
  同型機能が同じ経路で動作確認済みであることと、Kitty push/pop の
  ヘッドレステスト自体は両バックエンドで green であることから、
  低リスクと判断している。

## Wave 15 followup（実機確認で Shift+Enter がまだ効かない → 真因は別にあった）

Wave 15 の PR マージ後、実機（LaboLabo アプリ上で本物の `claude` を起動）
で確認したところ **Shift+Enter はまだ改行にならなかった**。Wave 15 時点の
「未検証」節に書いたとおり実機確認は最初から済んでいなかったため、これは
regression ではなく元々の未検証事項が実際に不具合だったと判明したケース。
4 つの仮説（クエリ応答の欠如／macOS のキー経路／ghostty 側の追跡／
その他）を総当たりで検証した結果、**当たっていたのはどれでもなかった**
-- 実際の Claude Code バイナリを逆解析して見つけた、より根本的な原因が
別にあった。

### 調査方法: 実際の Claude Code CLI バイナリを直接解析

`which claude` → `~/.local/share/claude/versions/2.1.212`（Bun でコンパイル
された単一 Mach-O 実行ファイル、245MB）。ソースは公開されていないが、
Bun バンドルは文字列リテラル・正規表現ソース・関数名（一部）がバイナリ内に
平文で残るため、`strings -a` (615,177 行) + `ripgrep` で該当ロジックを
直接読める。以下は実際に読んだコード片（変数名は minify 済みだが、
ロジックはそのまま引用）。

### 仮説①「クエリ応答（`CSI ? u`）が無いと push しない」-- 否定

Claude Code 自身のキーボード応答パーサ:

```js
Iwg = /^\x1b\[\?(\d+)u$/;          // クエリ応答 CSI ? <flags> u を解釈
C7i = /^\x1b\[(\d+)(?:;(\d+))?u/;  // 自分の stdin に来た CSI u キーイベントを解釈
```

`Iwg` は確かに存在するが、「クエリを送って応答を確認してから push する」
という分岐はどこにも無かった。push 自体は次の 1 行:

```js
Tiu = mE(">1u");   // "\x1b[>1u" -- kitty disambiguate を push
LLe = mE("<u");    // "\x1b[<u"  -- pop
function __e() { return YRg() ? LLe + Tiu + Siu : "" }
```

`__e()` は Ink（Claude Code の TUI ランタイム）が raw モードに入る/戻る
たびに**無条件で**呼ばれ、`YRg()` が真なら push 文字列を stdout（= この
アプリからは PTY 経由で子プロセスの stdin）へ書く。クエリ応答は一切
待たない。→ **仮説①は誤り。** ただし「対応バックエンド側は `CSI ? u`
に正しく応答できるか」自体は独立に価値のある確認なので、後述のとおり
新規 e2e テストで両バックエンドとも検証した（結果: 既存の `Event::
PtyWrite`/`on_pty_write`（DA1/DSR と同じ経路）がそのまま kitty query
にも効いており、**新規コード不要**だった）。

### 真因: `YRg()`（terminal-identity allowlist）が LaboLabo を認識していない

```js
KRg = ["iTerm.app", "kitty", "WezTerm", "ghostty", "tmux",
       "windows-terminal", "WarpTerminal"];
function YRg(e) { return KRg.includes(e ?? Z.terminal ?? "") }
```

`__e()`（kitty push を書く関数）は **`Z.terminal` がこの固定リストに
含まれるときしか呼ばれない**。つまり Claude Code は「相手の端末が Kitty
protocol を実装しているか」をライブに調べるのではなく、**静的な端末名
の allowlist** で判定している。`Z.terminal` の解決ロジック（優先順）:

```js
if (process.env.TERM === "xterm-ghostty") return "ghostty";
if (process.env.TERM?.includes("kitty")) return "kitty";
if (process.env.TERM_PROGRAM) { ...; return process.env.TERM_PROGRAM; }
if (process.env.TMUX) return "tmux";
// ... KITTY_WINDOW_ID / ALACRITTY_LOG / WT_SESSION / ...
```

Wave 15 以前（そして Wave 15 の変更後もそのまま）、`session.rs` が子
プロセスへ渡す環境変数は `TERM=xterm-256color` のみで、`TERM_PROGRAM`
は未設定だった。上のチェーンをどれも満たさないため `Z.terminal` は
Claude Code にとって「認識できない端末」のままになり、**`__e()` が一度も
呼ばれず、`\x1b[>1u` が子プロセスの stdin に一度も書き込まれない**。
Wave 15 で実装した `VtBackend::kitty_disambiguate`/CSI u エンコードは
「push が来れば正しく中継できる」よう terminal 側を正しくしただけで、
Claude Code 側がそもそも push を試みていなかった -- これが実機で
Shift+Enter が動かなかった直接の原因。

### 修正: 子プロセスに `TERM_PROGRAM=ghostty` を設定

`labolabo-term::session::TermSession::spawn_with_scrollback_options`
（`session.rs`）で、既存の `cmd.env("TERM", "xterm-256color")` に加えて
`cmd.env("TERM_PROGRAM", "ghostty")` を設定した。これだけで上の解決
チェーンの `TERM_PROGRAM` 分岐が `"ghostty"` を返し、`KRg` に一致する。

- **`TERM` ではなく `TERM_PROGRAM` を選んだ理由**: `TERM` は terminfo/
  termcap 解決に使われ、システムに `xterm-ghostty` terminfo が入って
  いない環境（実物の Ghostty を一度もインストールしていないマシンなど）
  では ncurses 系プログラム（`tput`、`vim` の一部機能など）を壊すリスク
  がある。`TERM_PROGRAM` は端末識別のための慣習的な変数で、それを読む
  プログラムはごく少数（端末機能検出目的のみ）のため、この用途には
  `TERM_PROGRAM` だけで十分かつ低リスク。
- **`"ghostty"` を名乗る妥当性**: 本クレートの本番バックエンドは実際に
  `libghostty-vt`（本物の Ghostty VT エンジン）そのものであり、CI 既定の
  `backend-alacritty` フォールバックも同じ統合テストで Ghostty 相当の
  挙動（bracketed paste・mouse mode・kitty keyboard 等）を保証している
  （`backend/mod.rs`）。したがってどちらのバックエンドでビルドされて
  いても "Ghostty 互換の端末である" という自己申告は正確。

### 仮説②「macOS で Shift+Enter が `key_down` に届いていない」-- 否定（ただし `keys.rs` 自身のコメントに誤りを発見）

`gpui` 0.2.2 の実ソースを直接読んで検証（`~/.cargo/registry/src/.../
gpui-0.2.2/`）。

- `platform/mac/events.rs`: Enter キーは `key_char` を**修飾キーに
  関係なく無条件で** `Some("\n")` にする
  (`Some(ENTER_KEY) => { key_char = Some("\n".to_string()); "enter" }`)。
  つまり Shift+Enter は `{ key: "enter", key_char: Some("\n"), modifiers:
  { shift: true, .. } }` になる -- `key_char` を持つ。
- `platform/mac/window.rs`'s `handle_key_event`: IME 合成中でなければ、
  `key_char` の有無に関わらず**まず `run_callback`（= `div::on_key_down`
  → `app::LaboLaboApp::key_down`）を呼ぶ**。`NSTextInputContext.
  handleEvent` は `run_callback` が `cx.stop_propagation()` しなかった
  場合の**フォールバック**としてのみ呼ばれる。

つまり macOS では `on_key_down` が `key_char` の有無に関わらず常に先に
呼ばれ、`keystroke_to_bytes` が `None` を返した（= plain な文字入力の
場合）ときだけ IME 側へフォールスルーする、という「先取り＋非取得なら
委譲」の構造だった。**`keys.rs` 自身の元々のモジュールドキュメントは
これと逆のこと**（"key_char を持つキーストロークは常に IME 側が横取り
し、`on_key_down` には届かない"）**を書いていた** -- 実際の観測結果
（素の Enter は元から動いていた）と矛盾しない誤解だったため気づかれずに
残っていたが、将来「gpui がどうせ横取りするから」という誤った前提で
このモジュールの防御的な分岐を削ってしまう regression の種になり得る
ため、コメントを実ソースの引用で修正した（`keys.rs` 参照）。→ **仮説②は
macOS について明確に否定**（X11/Wayland/Windows は今回未再監査 -- 実装が
プラットフォーム非依存の純関数である `keys.rs` 側の対応は変わらないが、
"gpui がどう配送するか" の部分はコメントで「未検証」と明記した）。

### 仮説③「ghostty バックエンドのフラグ追跡が効いていない」-- 否定

Wave 15 の PR #148 の実 CI ログを直接確認: `rust-term-ghostty
(macos-15)`・`rust-term-ghostty (ubuntu-latest)` 両ジョブのログに
`test kitty_disambiguate_reflects_csi_push_pop ... ok` が実際に出力
されている（`gh run view <run_id> --job <job_id> --log` で確認）。
**実 libghostty-vt バックエンドで push/pop の追跡は既に green** -- 憶測
ではなく CI ログの一次証跡で確定。

### 新規 e2e（両バックエンドで実行・green）

- `kitty_query_response_reflects_current_flags_before_and_after_push`
  （`labolabo-term/tests/backend_common.rs`）: 子プロセスが `stty raw
  -echo` の上で `CSI ? u` を送り、応答を `dd bs=1 count=5` で生バイト
  キャプチャ（このクレート自身の VT パーサを経由させない、W5j の mouse
  e2e と同じ手法）。push 前は `\x1b[?0u`、`CSI > 1 u` push 後は
  `\x1b[?1u` が返ることを両バックエンドで確認 -- 仮説①の「クエリ応答は
  新規実装不要」を実測で裏付け。
- `term_program_env_is_ghostty_for_every_spawned_child`
  （同ファイル）: 子プロセスの `$TERM_PROGRAM` が実際に `ghostty` に
  なっていることを確認 -- 今回の修正そのものの直接的な回帰テスト。
- `real_macos_enter_key_char_does_not_change_the_result`
  （`labolabo-app/src/keys.rs`）: 実 macOS が生成する `key_char:
  Some("\n")` を持つ Shift+Enter キーストロークでも結果が変わらない
  ことを確認 -- 仮説②の裏付け。

### 品質ゲート結果

- `cargo build/test/clippy(-D warnings)` を `-p labolabo-term`（alacritty
  backend（既定）と ghostty-vt backend（`PATH` に
  `~/.local/opt/zig-aarch64-macos-0.16.0` を先頭追加 +
  `GHOSTTY_SOURCE_DIR=~/ghq/.../labolabo-spikes/ghostty-zig016-src` で
  ローカル実ビルド）の両方）/ `-p labolabo-app` / ルート
  `default-members` それぞれで実行、全 green（`labolabo-term`: unit 6 +
  統合 28、うち新規 2 件込み。`labolabo-app`: `keys::` 18 -- 前回 17 +
  新規 1）。
- `cargo fmt --check`（ワークスペース全体）green。
- スモーク: `LABOLABO_RS_DATA_DIR=$(mktemp -d)` の上で `cargo run -p
  labolabo-app` を約5秒起動して kill -- パニック/クラッシュ出力なし
  （`timeout(1)` がこのマシンに無いため、バックグラウンド起動 + `sleep
  5` + `kill` で代替。合成入力・spawn直後のデフォルト状態 assert なし）。

### 手動確認手順（PR に記載・実機の `claude` 確認前にこれで一次確認可能）

```sh
# LaboLabo で新規ペインを開き、シェルで直接:
printf 'TERM_PROGRAM=%s\n' "$TERM_PROGRAM"   # -> "ghostty" になっているはず
stty raw -echo; printf '\033[?u'; dd bs=1 count=5 2>/dev/null | cat -v; stty sane; echo
# -> "^[[?0u" (プッシュ前、flags=0) が出力されるはず
stty raw -echo; printf '\033[>1u\033[?u'; dd bs=1 count=5 2>/dev/null | cat -v; stty sane; echo
# -> "^[[?1u" (disambiguate を push 後、flags=1) が出力されるはず
```

### 未検証

- **実際の `claude` での Shift+Enter 改行は依然ユーザー未確認。** 上記は
  すべて「Claude Code の実バイナリを逆解析して判明した判定ロジック」＋
  「そのロジックが要求する条件（`TERM_PROGRAM=ghostty`）をこのアプリが
  満たすようになったこと」をヘッドレスな e2e テストで裏付けたものであり、
  実機の LaboLabo アプリ上で `claude` を起動し、実際に Shift+Enter を
  押して改行が入ることそのものはまだ確認していない（合成キーボード入力は
  この移植の検証方針で禁止されているため、ユーザー確認が必要）。ただし
  今回はブラックボックスの推測ではなく、Claude Code 自身のバイナリが
  実際にチェックしている条件をソースレベルで特定し、それを満たしたという
  点で、Wave 15 時点より確度は大きく上がっている。
- **X11/Wayland/Windows での `on_key_down` 配送順は今回未検証**（macOS の
  gpui ソースのみ直接確認）。`keys.rs` はプラットフォーム非依存の純関数
  なので機能的な影響は無いはずだが、"gpui 側がどう配送するか" は
  プラットフォームごとに独立した検証が必要な部分として未確認のまま
  残した。
- 実際の Claude Code バイナリは version 2.1.212 時点のもの -- 将来
  バージョンで `KRg` allowlist や `Z.terminal` 解決ロジックが変わる
  可能性はあり、その場合はこの節の記述も追随が必要。
