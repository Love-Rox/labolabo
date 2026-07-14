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

The AF_UNIX transport (`UnixSocketEventTransport`) and the forwarder
(`forward_hook`, `src/bin/labolabo-hook.rs`) are `#[cfg(unix)]` — a Windows
transport (Named Pipe, per docs/hooks-protocol.md §4) is future work with no
stub yet, just a comment. This introduces the crate's first genuinely
platform-specific code and its first target-specific dependency: `libc`
(unix-only, `[target.'cfg(unix)'.dependencies]`), needed for `shutdown(2)`
on a raw fd to unblock a blocked `accept()` call from another thread when
`stop()` is called — `std::os::unix::net::UnixListener` exposes no such
method.

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
- `src/hooks.rs`'s `#[cfg(all(test, unix))] mod unix_bus_tests`: the real
  AF_UNIX round-trip, ported 1:1 from all 6 tests in
  `Tests/LaboLaboEngineTests/AgentStatusBusTests.swift` (a real POSIX
  client connects and sends one payload per connection; `on_event`
  fires/doesn't fire with the right `AgentStatusEvent`).
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
    Cargo.toml                      # serde_json is a runtime dep (wave 2); serde's derive feature too (wave 3); libc (unix-only) too (wave 4b)
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
      util.rs                       # small string helpers shared by the parsers
      hooks.rs                      # wave 4b: port of AgentStatusBus.swift + HookForwarder.swift + unit tests
      bin/
        labolabo-hook.rs             # wave 4b: thin `labolabo-hook <socket>` forwarder binary
    tests/
      golden.rs                     # golden-oracle test (see below; wave 1/2 modules only)
      tiling_golden.rs               # wave 3: tiling's own golden test (separate oracle mechanism, see below)
      labolabo_hook_bin.rs           # wave 4b: end-to-end test spawning the real labolabo-hook binary
    fixtures/
      generate.swift                # the Swift-side "oracle" generator (see below; wave 1/2 modules only)
      tiling/*.json                 # wave 3: real JSONEncoder output for TileLayout (separate oracle, see below)
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

- `GitRunner`/`GitEngine` (process execution + orchestration), `FileWatcher`,
  and persistence (`LaboLaboStore`) remain unported and out of scope. The
  `AgentStatusBus`/`AgentEventTransport` socket-transport layer was ported in
  wave 4b (see above) -- the settings.local.json hooks-injection app-layer
  logic (`app/Sources/AgentSessionModel.swift`) that creates
  `/tmp/labolabo` and merges/restores the worktree's `.claude/settings.local.json`
  is still unported (app-layer, not engine-layer, same split as the Swift
  source).
- `commit_graph::build`'s only consumer in Swift,
  `GitEngine.commitGraph(worktree:limit:)`, is process execution and is not
  ported — a future wave that ports `GitRunner`/`GitEngine` would need to
  add a thin `git log` invocation wrapper around `commit_graph::build`.
- Golden coverage exists for `porcelain`, `unified_diff`, `worktree`,
  `transcript_usage`, `agent_event_parser`, and `tiling` (the last via its
  own `tests/tiling_golden.rs`, not `tests/golden.rs`). `commit_graph`,
  `cross_session_conflicts`, and `release_version` are covered by ported
  unit tests only (no golden fixtures), by design — see "Wave 2" above.
- `tiling::PaneTilingActions` is a trait with no production implementation
  yet (no Rust UI layer exists to implement it against) — only a
  test-only mock (`tiling::tests::MockCoordinator`).
