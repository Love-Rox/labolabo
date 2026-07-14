# labolabo-core (Rust)

Wave 1 of the Rust cross-platform migration: a faithful port of
LaboLaboEngine's pure-logic core — the parsers with no process /
filesystem / concurrency dependencies — from Swift to Rust.

Ported so far (from `Sources/LaboLaboEngine/Git/` in the Swift package):

| Swift source | Rust module |
|---|---|
| `GitModels.swift` | `crates/labolabo-core/src/git_models.rs` |
| `PorcelainStatusParser.swift` | `crates/labolabo-core/src/porcelain.rs` |
| `UnifiedDiffParser.swift` | `crates/labolabo-core/src/unified_diff.rs` |

Everything else in `LaboLaboEngine` (process execution, git plumbing that
shells out, file watching, hooks/agent status, persistence, ...) is out of
scope for this wave.

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

## Workspace layout

```
rust/
  Cargo.toml                        # workspace, resolver = "2"
  crates/labolabo-core/
    Cargo.toml                      # no runtime deps; serde/serde_json are dev-dependencies only
    src/
      lib.rs
      git_models.rs                 # port of GitModels.swift + unit tests ported from Swift XCTest
      porcelain.rs                  # port of PorcelainStatusParser.swift + unit tests
      unified_diff.rs               # port of UnifiedDiffParser.swift + unit tests
      util.rs                       # small string helpers shared by the two parsers
    tests/
      golden.rs                     # golden-oracle test (see below)
    fixtures/
      generate.swift                # the Swift-side "oracle" generator (see below)
      inputs/
        porcelain/*.txt, *.raw      # git status --porcelain=v2 -z inputs
        diff/*.diff                 # git diff inputs
      expected/
        porcelain/*.json            # canonical JSON produced by the *Swift* parser
        diff/*.json
```

## Golden-oracle testing

Correctness is anchored to the Swift implementation: `tests/golden.rs` reads
every fixture under `fixtures/inputs/{porcelain,diff}/`, runs it through the
Rust parsers, renders a canonical JSON view, and asserts it is
**byte-identical** to the corresponding file under
`fixtures/expected/{porcelain,diff}/` — which was produced by running the
*same* fixture files through the real Swift parsers.

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

### Regenerating `fixtures/expected/**`

`fixtures/expected/**` must be regenerated any time a fixture is
added/changed *or* the canonical JSON schema changes. It is produced by a
disposable Swift "oracle" script, `fixtures/generate.swift`, that imports
the real `LaboLaboEngine` module and runs the two Swift parsers over every
file in `fixtures/inputs/`.

`fixtures/generate.swift` is **not** part of the SwiftPM package graph — no
executable target was added to `Package.swift` (the porting brief
explicitly disallows that, to keep the Swift package's own structure
untouched). Instead it links directly against the already-built object
files for `GitModels`/`PorcelainStatusParser`/`UnifiedDiffParser` (which
depend on nothing outside Foundation) and is compiled as an ordinary
one-off `swiftc` binary. From the repo root:

```sh
# 1. Make sure LaboLaboEngine is built (produces the .o files we link against).
swift build

# 2. Compile the oracle script against those object files.
BUILD=.build/arm64-apple-macosx/debug   # adjust triple if not on arm64 macOS
swiftc -O -I "$BUILD/Modules" rust/crates/labolabo-core/fixtures/generate.swift \
  "$BUILD/LaboLaboEngine.build/GitModels.swift.o" \
  "$BUILD/LaboLaboEngine.build/PorcelainStatusParser.swift.o" \
  "$BUILD/LaboLaboEngine.build/UnifiedDiffParser.swift.o" \
  -o /tmp/labolabo_golden_generate

# 3. Run it, pointing at the fixtures directory. It (re)writes every file
#    under fixtures/expected/{porcelain,diff}/*.json.
/tmp/labolabo_golden_generate rust/crates/labolabo-core/fixtures
```

This leaves zero footprint in `Sources/` or `Tests/` — nothing to add and
then delete before committing. (If this trick ever stops working on some
future toolchain, the brief's documented fallback is to temporarily add a
`#if GOLDEN_EXPORT`-guarded test to `Tests/LaboLaboEngineTests/`, run it via
`swift test --filter`, then delete it before committing — the JSON-shape
logic in `generate.swift` can be pasted in as-is.)

After regenerating, run `cd rust && cargo test` — if the fixture or schema
changed, the two golden tests in `tests/golden.rs` will fail with a
byte-diff-style message naming the mismatched fixture.

## Verification run for this wave

```sh
cd rust
cargo test                    # unit tests (ported from Swift XCTest) + golden tests
cargo fmt --check
cargo clippy --all-targets -- -D warnings

cd ..
swift test                    # confirms the Swift side is untouched and still green
```

## Known scope limits / what's next

- No `rust` job exists in CI yet (`.github/workflows/ci.yml`) — that's
  planned for a later wave, once more of the engine is ported. This crate
  currently has to be built/tested manually as above.
- Only the pure parsers are ported. `GitRunner`/`GitEngine` (process
  execution + orchestration), `FileWatcher`, hooks/agent-status, and
  persistence (`LaboLaboStore`) are unported and out of scope for this wave.
