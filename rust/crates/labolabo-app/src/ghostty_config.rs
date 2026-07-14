//! Reads the user's Ghostty configuration -- just enough of it to style this
//! app's terminal like the user's own Ghostty (`font-family`, `font-size`).
//!
//! ## What this is (and is not)
//!
//! A minimal, faithful port of the slice of Ghostty's config loading that
//! matters for font extraction, cross-checked against the actual Ghostty
//! source (the `ghostty-zig016-src` checkout, upstream PR #12726 state):
//!
//! - default file discovery: `src/config/file_load.zig` + `Config.loadDefaultFiles`
//! - line syntax: `src/cli/args.zig` (`LineIterator`)
//! - `config-file` includes: `Config.loadRecursiveFiles` + `src/config/path.zig`
//!
//! It is **not** a general Ghostty config parser: keys other than
//! `font-family`, `font-size`, and `config-file` are read and skipped.
//! Colors (`background`/`foreground`/`palette`) are deliberately out of
//! scope for this wave -- see the crate README's TODO for what supporting
//! them needs from `labolabo-term`.
//!
//! ## Canonical behavior ported (with source references)
//!
//! **Default files, in load order -- later wins** (`loadDefaultFiles`):
//!
//! 1. `$XDG_CONFIG_HOME/ghostty/config` (legacy, Ghostty <1.3.0;
//!    `~/.config` when `XDG_CONFIG_HOME` is unset/empty)
//! 2. `$XDG_CONFIG_HOME/ghostty/config.ghostty` (>=1.3.0)
//! 3. macOS only: `~/Library/Application Support/com.mitchellh.ghostty/config`
//! 4. macOS only: `~/Library/Application Support/com.mitchellh.ghostty/config.ghostty`
//!
//! Upstream loads *all* of these that exist (warning when both a legacy and
//! a new file exist, but still loading both, legacy first). A default file
//! is skipped unless it exists, is a regular file, and is **non-empty**
//! (`file_load.open` treats an empty file as an error; included files, by
//! contrast, are opened directly and an empty include is a silent no-op).
//! Upstream's `preferredAppSupportPath` double-load dance collapses to the
//! same observable result as "try both fixed paths in order, skipping
//! non-loadable ones", which is what we do. (Upstream also *creates* a
//! template config when none exists -- we are read-only and never do.)
//!
//! **Line syntax** (`LineIterator.next`): each line is trimmed of spaces,
//! tabs, and `\r`; blank lines and lines whose first character is `#` are
//! skipped (no trailing comments); the *first* `=` splits key from value;
//! both are trimmed of spaces/tabs; a value fully wrapped in double quotes
//! has exactly one quote layer stripped. A UTF-8 BOM at the start of a file
//! is skipped (`loadReader`).
//!
//! **Key semantics**: `font-family` is a *repeatable* value -- each
//! occurrence appends a fallback family, and an *empty* value resets the
//! accumulated list (`RepeatableString.parseCLI`). `font-size` is a float;
//! the last valid value wins. A line with no `=` never carries a value, so
//! for these keys it is a no-op (upstream: `error.ValueRequired` becomes a
//! diagnostic and the value is left unchanged).
//!
//! **`config-file` includes** (`loadRecursiveFiles` + `path.zig`): values
//! accumulate while files load, but are only *processed* after every root
//! file has loaded, in order, appending recursively discovered includes to
//! the back of the queue -- so an include's settings override every root
//! file's, no matter which root declared the include. A leading `?` marks
//! the include optional (missing file silently skipped; a missing required
//! include just logs -- either way loading continues). One layer of double
//! quotes is stripped *after* the `?` check. Relative paths resolve against
//! the *including* file's directory; `~/` resolves against the home
//! directory (`Path.expand`). An empty path is skipped. A path already seen
//! *as an include* is skipped ("cycle detected") -- faithfully to upstream,
//! the root files themselves are **not** in that visited set, so a root
//! that includes itself genuinely gets loaded twice (double-appending any
//! repeatable values) before the cycle check stops the third pass.
//!
//! ## Testability
//!
//! Everything below `load_user_font_config` is pure with respect to the
//! environment: discovery takes `home`/`xdg_config_home` as parameters and
//! loading takes explicit root paths, so the unit tests run entirely on
//! fixture trees under `fixtures/ghostty_config/` -- no test reads `$HOME`
//! or the real user's config.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The font-relevant subset of a Ghostty configuration.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontConfig {
    /// `font-family` entries, in order (first = primary, rest = fallbacks
    /// in Ghostty; see `render::RenderSpec` for how this app uses them).
    /// Empty when the user never set one.
    pub families: Vec<String>,
    /// `font-size` in points, if set to a parseable value.
    pub size: Option<f32>,
}

/// Ghostty's per-OS `font-size` default (`Config.zig`: 13 on macOS, 12
/// elsewhere).
pub fn default_font_size() -> f32 {
    if cfg!(target_os = "macos") {
        13.0
    } else {
        12.0
    }
}

/// The characters Ghostty's config-line trimming removes around keys and
/// values (`cli/args.zig`: `whitespace = " \t"`).
const WHITESPACE: &[char] = &[' ', '\t'];

/// One parsed config line: a key plus its (possibly absent) value.
/// `value: None` means the line had no `=` at all -- distinct from
/// `Some("")` (an explicit empty value), which e.g. resets `font-family`.
#[derive(Debug, PartialEq, Eq)]
struct ConfigLine<'a> {
    key: &'a str,
    value: Option<&'a str>,
}

/// Parse one line per `LineIterator.next`'s rules. Returns `None` for blank
/// lines and full-line `#` comments.
fn parse_line(raw: &str) -> Option<ConfigLine<'_>> {
    let line = raw.trim_matches(|c| c == ' ' || c == '\t' || c == '\r');
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    match line.split_once('=') {
        None => Some(ConfigLine {
            key: line,
            value: None,
        }),
        Some((key, value)) => {
            let key = key.trim_matches(WHITESPACE);
            let mut value = value.trim_matches(WHITESPACE);
            // One quote layer is stripped when the value is fully wrapped
            // in double quotes (LineIterator: `len >= 2`, so a lone `"`
            // survives as-is).
            if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
                value = &value[1..value.len() - 1];
            }
            Some(ConfigLine {
                key,
                value: Some(value),
            })
        }
    }
}

/// A `config-file` include: the (possibly still relative) path plus whether
/// a leading `?` marked it optional.
#[derive(Debug, PartialEq, Eq)]
struct IncludeRef {
    path: String,
    optional: bool,
}

/// Parse a `config-file` *value* per `Path.parse` (`path.zig`): leading `?`
/// = optional, then one more quote layer stripped (so a config file can
/// express a literal-`?` path as `""?path""` -- the line parser strips the
/// outer layer, this strips the inner). Empty (before or after stripping)
/// means "no include" (`RepeatablePath.parseCLI` appends nothing).
fn parse_include_value(value: &str) -> Option<IncludeRef> {
    if value.is_empty() {
        return None;
    }
    let (optional, rest) = match value.strip_prefix('?') {
        Some(rest) => (true, rest),
        None => (false, value),
    };
    let rest = if rest.len() >= 2 && rest.starts_with('"') && rest.ends_with('"') {
        &rest[1..rest.len() - 1]
    } else {
        rest
    };
    if rest.is_empty() {
        return None;
    }
    Some(IncludeRef {
        path: rest.to_string(),
        optional,
    })
}

/// Resolve an include path against the including file's directory
/// (`Config.expandPaths` runs with `base = dirname(containing file)`) and
/// expand a leading `~/` against `home` (`Path.expand` + `expandHomeUnix`;
/// `~user/` forms are *not* supported, same as upstream).
fn resolve_include_path(path: &str, base_dir: &Path, home: &Path) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        return home.join(rest);
    }
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base_dir.join(p)
    }
}

/// Whether a *default* (root) config file would be loaded by upstream's
/// `file_load.open`: exists, is a regular file, and is non-empty.
fn is_loadable_root(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(meta) => meta.is_file() && meta.len() > 0,
        Err(_) => false,
    }
}

/// The default config file paths in Ghostty's load order (later files win).
/// Pure: `home` and `xdg_config_home` are parameters, and no existence
/// filtering happens here -- the loader skips what it can't load.
pub fn default_config_paths(
    home: &Path,
    xdg_config_home: Option<&Path>,
    include_app_support: bool,
) -> Vec<PathBuf> {
    let xdg_base = match xdg_config_home {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => home.join(".config"),
    };
    let mut paths = vec![
        xdg_base.join("ghostty/config"),
        xdg_base.join("ghostty/config.ghostty"),
    ];
    if include_app_support {
        let dir = home.join("Library/Application Support/com.mitchellh.ghostty");
        paths.push(dir.join("config"));
        paths.push(dir.join("config.ghostty"));
    }
    paths
}

/// Read one config file's bytes as (lossy) UTF-8 with any leading BOM
/// stripped. `None` if the file can't be read.
fn read_config_text(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let bytes = bytes
        .strip_prefix(&[0xef, 0xbb, 0xbf][..])
        .unwrap_or(&bytes);
    Some(String::from_utf8_lossy(bytes).into_owned())
}

/// Accumulating parse state across all loaded files.
#[derive(Default)]
struct Loader {
    fonts: FontConfig,
    /// The `config-file` include queue, in declaration order. Grows while
    /// includes are being processed (recursive includes append here).
    includes: Vec<(PathBuf, bool)>,
}

impl Loader {
    /// Parse one file's text: apply font keys, queue `config-file` values.
    fn load_text(&mut self, text: &str, base_dir: &Path, home: &Path) {
        for raw in text.lines() {
            let Some(line) = parse_line(raw) else {
                continue;
            };
            match line.key {
                "font-family" => match line.value {
                    // No `=` on the line: upstream records a
                    // "value required" diagnostic and changes nothing.
                    None => {}
                    // Empty value resets the accumulated list.
                    Some("") => self.fonts.families.clear(),
                    Some(v) => self.fonts.families.push(v.to_string()),
                },
                "font-size" => {
                    if let Some(v) = line.value {
                        match v.parse::<f32>() {
                            Ok(size) if size.is_finite() => self.fonts.size = Some(size),
                            _ => eprintln!(
                                "labolabo-app: ignoring unparseable font-size {v:?} in ghostty config"
                            ),
                        }
                    }
                }
                "config-file" => {
                    if let Some(v) = line.value {
                        if let Some(inc) = parse_include_value(v) {
                            self.includes.push((
                                resolve_include_path(&inc.path, base_dir, home),
                                inc.optional,
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Load one file from disk (any file type/emptiness checking is the
    /// caller's job -- mirrors upstream, where root files and includes have
    /// *different* checks).
    fn load_file(&mut self, path: &Path, home: &Path) {
        let Some(text) = read_config_text(path) else {
            return;
        };
        let base_dir = path.parent().unwrap_or(Path::new("/")).to_path_buf();
        self.load_text(&text, &base_dir, home);
    }
}

/// Load the given root config files (in order) and then their `config-file`
/// includes (upstream `loadDefaultFiles` + `loadRecursiveFiles` semantics
/// -- see the module doc comment), returning the extracted font settings.
pub fn extract_font_config(roots: &[PathBuf], home: &Path) -> FontConfig {
    let mut loader = Loader::default();

    for root in roots {
        if is_loadable_root(root) {
            loader.load_file(root, home);
        }
    }

    // Process the include queue. Faithful quirks: the queue grows while
    // being walked (recursive includes go to the back); the visited set
    // covers only include entries, not the roots.
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut i = 0;
    while i < loader.includes.len() {
        let (path, optional) = loader.includes[i].clone();
        i += 1;

        if !visited.insert(path.clone()) {
            eprintln!(
                "labolabo-app: config-file {}: cycle detected",
                path.display()
            );
            continue;
        }
        let meta = match fs::metadata(&path) {
            Ok(meta) => meta,
            Err(err) => {
                if !optional || err.kind() != std::io::ErrorKind::NotFound {
                    eprintln!(
                        "labolabo-app: error opening config-file {}: {err}",
                        path.display()
                    );
                }
                continue;
            }
        };
        if !meta.is_file() {
            eprintln!(
                "labolabo-app: config-file {}: not reading because it is not a file",
                path.display()
            );
            continue;
        }
        // Note: no emptiness check here -- an empty *include* is a no-op
        // upstream (only default root files reject empty).
        loader.load_file(&path, home);
    }

    loader.fonts
}

/// Read the real user's Ghostty font configuration: `$HOME` /
/// `$XDG_CONFIG_HOME` discovery, macOS Application Support included on
/// macOS. Missing config files (or no `$HOME` at all) just mean defaults.
pub fn load_user_font_config() -> FontConfig {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return FontConfig::default();
    };
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let roots = default_config_paths(&home, xdg.as_deref(), cfg!(target_os = "macos"));
    extract_font_config(&roots, &home)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/ghostty_config")
    }

    // --- parse_line -------------------------------------------------------

    #[test]
    fn blank_and_comment_lines_are_skipped() {
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line("   \t "), None);
        assert_eq!(parse_line("# a comment"), None);
        assert_eq!(parse_line("  \t# indented comment"), None);
    }

    #[test]
    fn trailing_comments_are_not_a_thing() {
        // Upstream only skips lines whose first character is '#'; a '#'
        // later in the line is part of the value.
        let line = parse_line("font-family = Menlo # not a comment").unwrap();
        assert_eq!(line.key, "font-family");
        assert_eq!(line.value, Some("Menlo # not a comment"));
    }

    #[test]
    fn key_and_value_are_trimmed_and_split_at_the_first_equals() {
        let line = parse_line("  font-size\t=  13.5  ").unwrap();
        assert_eq!(line.key, "font-size");
        assert_eq!(line.value, Some("13.5"));

        // First '=' wins; later ones belong to the value.
        let line = parse_line("key = a = b").unwrap();
        assert_eq!(line.key, "key");
        assert_eq!(line.value, Some("a = b"));
    }

    #[test]
    fn one_quote_layer_is_stripped_from_the_value() {
        let line = parse_line("font-family = \"Fira Code\"").unwrap();
        assert_eq!(line.value, Some("Fira Code"));

        // `""` is a quoted empty string -> empty value.
        let line = parse_line("font-family = \"\"").unwrap();
        assert_eq!(line.value, Some(""));

        // A lone quote (len 1) is not a wrapped value.
        let line = parse_line("font-family = \"").unwrap();
        assert_eq!(line.value, Some("\""));

        // Only ONE layer comes off.
        let line = parse_line("key = \"\"double\"\"").unwrap();
        assert_eq!(line.value, Some("\"double\""));
    }

    #[test]
    fn a_line_without_equals_has_no_value() {
        let line = parse_line("some-flag").unwrap();
        assert_eq!(line.key, "some-flag");
        assert_eq!(line.value, None);
    }

    #[test]
    fn crlf_line_endings_are_trimmed() {
        let line = parse_line("font-size = 14\r").unwrap();
        assert_eq!(line.value, Some("14"));
    }

    // --- parse_include_value / resolve_include_path ------------------------

    #[test]
    fn include_value_parses_optional_prefix_and_quotes() {
        assert_eq!(
            parse_include_value("extra.conf"),
            Some(IncludeRef {
                path: "extra.conf".into(),
                optional: false
            })
        );
        assert_eq!(
            parse_include_value("?missing.conf"),
            Some(IncludeRef {
                path: "missing.conf".into(),
                optional: true
            })
        );
        // Quote layer strips AFTER the `?` check (path.zig order).
        assert_eq!(
            parse_include_value("?\"quoted.conf\""),
            Some(IncludeRef {
                path: "quoted.conf".into(),
                optional: true
            })
        );
        assert_eq!(parse_include_value(""), None);
        assert_eq!(parse_include_value("?"), None);
        assert_eq!(parse_include_value("\"\""), None);
    }

    #[test]
    fn include_paths_resolve_relative_tilde_and_absolute() {
        let base = Path::new("/base/dir");
        let home = Path::new("/home/u");
        assert_eq!(
            resolve_include_path("sub/x.conf", base, home),
            PathBuf::from("/base/dir/sub/x.conf")
        );
        assert_eq!(
            resolve_include_path("~/x.conf", base, home),
            PathBuf::from("/home/u/x.conf")
        );
        assert_eq!(
            resolve_include_path("/abs/x.conf", base, home),
            PathBuf::from("/abs/x.conf")
        );
        // `~user/` is NOT expanded (upstream only handles `~/`): it stays a
        // relative path against the base dir.
        assert_eq!(
            resolve_include_path("~other/x.conf", base, home),
            PathBuf::from("/base/dir/~other/x.conf")
        );
    }

    // --- default_config_paths ----------------------------------------------

    #[test]
    fn default_paths_use_xdg_config_home_when_set() {
        let paths = default_config_paths(Path::new("/home/u"), Some(Path::new("/xdg")), false);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/xdg/ghostty/config"),
                PathBuf::from("/xdg/ghostty/config.ghostty"),
            ]
        );
    }

    #[test]
    fn default_paths_fall_back_to_dot_config_and_add_app_support_on_macos() {
        let paths = default_config_paths(Path::new("/home/u"), None, true);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/home/u/.config/ghostty/config"),
                PathBuf::from("/home/u/.config/ghostty/config.ghostty"),
                PathBuf::from("/home/u/Library/Application Support/com.mitchellh.ghostty/config"),
                PathBuf::from(
                    "/home/u/Library/Application Support/com.mitchellh.ghostty/config.ghostty"
                ),
            ]
        );
    }

    #[test]
    fn empty_xdg_config_home_is_treated_as_unset() {
        let paths = default_config_paths(Path::new("/home/u"), Some(Path::new("")), false);
        assert_eq!(paths[0], PathBuf::from("/home/u/.config/ghostty/config"));
    }

    // --- extract_font_config on fixtures ------------------------------------

    #[test]
    fn basic_fixture_parses_families_and_size() {
        let root = fixture_root().join("basic/config");
        let cfg = extract_font_config(&[root], &fixture_root());
        // The fixture appends two families, resets, then appends the final
        // two -- exercising append + empty-value reset. Invalid font-size
        // values are ignored; the last valid one wins.
        assert_eq!(cfg.families, vec!["JetBrains Mono", "Symbols Nerd Font"]);
        assert_eq!(cfg.size, Some(15.5));
    }

    #[test]
    fn bom_is_stripped() {
        let root = fixture_root().join("bom/config");
        let cfg = extract_font_config(&[root], &fixture_root());
        assert_eq!(cfg.families, vec!["BOM Font"]);
    }

    #[test]
    fn nonexistent_and_empty_roots_are_silently_skipped() {
        let missing = fixture_root().join("does-not-exist/config");
        let empty = fixture_root().join("empty/config");
        let cfg = extract_font_config(&[missing, empty], &fixture_root());
        assert_eq!(cfg, FontConfig::default());
    }

    #[test]
    fn later_root_overrides_earlier_scalar_and_appends_repeatable() {
        // Simulates XDG + Application Support both existing: both load, in
        // order; the later file's font-size wins, and its font-family
        // *appends* (repeatable) to the earlier file's unless reset.
        let roots = vec![
            fixture_root().join("two_roots/xdg_config"),
            fixture_root().join("two_roots/app_support_config"),
        ];
        let cfg = extract_font_config(&roots, &fixture_root());
        assert_eq!(cfg.families, vec!["XdgFont", "AppSupportFont"]);
        assert_eq!(cfg.size, Some(20.0));
    }

    #[test]
    fn includes_load_after_all_roots_and_override_them() {
        // Root A declares an include whose font-size conflicts with root
        // B's. Upstream semantics: the include is processed after BOTH
        // roots, so it wins even though root B loaded after root A.
        let roots = vec![
            fixture_root().join("include_after_roots/root_a"),
            fixture_root().join("include_after_roots/root_b"),
        ];
        let cfg = extract_font_config(&roots, &fixture_root());
        assert_eq!(cfg.size, Some(99.0));
    }

    #[test]
    fn includes_resolve_relative_to_the_including_file() {
        let root = fixture_root().join("include_tree/config");
        let cfg = extract_font_config(&[root], &fixture_root());
        // config includes sub/extra.conf (relative), which includes
        // nested.conf relative to sub/ -- both must resolve and load, and
        // the `?missing.conf` optional include must be silently skipped.
        assert_eq!(cfg.families, vec!["RootFont", "SubFont", "NestedFont"]);
        assert_eq!(cfg.size, Some(17.0));
    }

    #[test]
    fn tilde_includes_resolve_against_home() {
        // `home` is injected, so the fixture's `~/home_font.conf` resolves
        // inside the fixture tree -- no real $HOME involved.
        let home = fixture_root().join("tilde/home");
        let root = fixture_root().join("tilde/config");
        let cfg = extract_font_config(&[root], &home);
        assert_eq!(cfg.families, vec!["HomeFont"]);
    }

    #[test]
    fn include_cycles_are_detected_and_do_not_hang() {
        // a.conf includes b.conf which includes a.conf again. Faithful to
        // upstream: a.conf is a *root* here, so its re-inclusion loads it a
        // second time (families appear twice) before the visited set stops
        // the third pass.
        let root = fixture_root().join("cycle/a.conf");
        let cfg = extract_font_config(&[root], &fixture_root());
        assert_eq!(cfg.families, vec!["FontA", "FontB", "FontA"]);
    }

    #[test]
    fn missing_required_include_is_logged_but_loading_continues() {
        let root = fixture_root().join("missing_required/config");
        let cfg = extract_font_config(&[root], &fixture_root());
        // The missing include didn't abort the rest of the file's settings
        // (which were already applied) nor the queue.
        assert_eq!(cfg.families, vec!["StillHere"]);
    }

    #[test]
    fn empty_include_is_a_noop_not_an_error() {
        // An empty *included* file loads as a no-op (unlike an empty root,
        // which is skipped by the file_load.open emptiness check).
        let root = fixture_root().join("empty_include/config");
        let cfg = extract_font_config(&[root], &fixture_root());
        assert_eq!(cfg.families, vec!["Before", "After"]);
    }
}
