//! Reads the user's Ghostty configuration -- just enough of it to style this
//! app's terminal like the user's own Ghostty (`font-family`/`font-size`,
//! and `background`/`foreground`/`cursor-color`/`palette`/`theme`).
//!
//! ## What this is (and is not)
//!
//! A minimal, faithful port of the slice of Ghostty's config loading that
//! matters for font and color extraction, cross-checked against the actual
//! Ghostty source (the `ghostty-zig016-src` checkout, upstream PR #12726
//! state):
//!
//! - default file discovery: `src/config/file_load.zig` + `Config.loadDefaultFiles`
//! - line syntax: `src/cli/args.zig` (`LineIterator`)
//! - `config-file` includes: `Config.loadRecursiveFiles` + `src/config/path.zig`
//! - color value syntax: `terminal/color.zig`'s `RGB.parse` (a supported
//!   subset -- see "Color value syntax" below) and `parsePaletteEntry`
//! - theme resolution: `src/config/theme.zig` + `Config.zig`'s `loadTheme`/
//!   `finalize` (see "Theme resolution" below)
//!
//! It is **not** a general Ghostty config parser: keys other than
//! `font-family`, `font-size`, `config-file`, `background`, `foreground`,
//! `cursor-color`, `palette`, and `theme` are read and skipped.
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
//! **Color keys**: `background`/`foreground`/`cursor-color` are scalar --
//! the last *parseable* value wins, same as `font-size` (an unparseable
//! value is reported and the previous value, if any, is left alone;
//! upstream: `Color.parseCLI`/`TerminalColor.parseCLI` return
//! `error.InvalidValue`, which becomes a diagnostic, not a config change).
//! `palette` is repeatable in the form `N=COLOR` (`N` = 0-255, decimal or
//! `0x`/`0o`/`0b`-prefixed -- `terminal.color.parsePaletteEntry`); each
//! occurrence sets exactly that one palette index, so setting the same
//! index twice just overwrites it (there is no "reset the whole palette"
//! form, unlike `font-family`'s empty-value reset).
//!
//! **Color value syntax** (`terminal.color.RGB.parse`, subset): `#rgb`,
//! `#rrggbb`, and the same two forms without the leading `#` (Ghostty
//! accepts bare hex for config/theme compatibility -- confirmed against a
//! real user theme file, which used exactly this form). Upstream also
//! accepts `rgb:h/hh/hhh/hhhh` device syntax, `rgbi:<float>/<float>/<float>`,
//! the 12-/16-bit-per-channel `#rrrgggbbb`/`#rrrrggggbbbb` forms, and ~750
//! X11 named colors (`terminal/x11_color.zig`) -- all deliberately
//! unsupported here (reported and skipped, previous value untouched). A
//! scan of all 463 themes bundled with a real Ghostty.app install found
//! zero uses of any of these unsupported forms, so the supported subset
//! covers every built-in theme and the overwhelming majority of
//! hand-written configs.
//!
//! **Theme resolution** (`Config.zig`'s `loadTheme` + `finalize`, and
//! `theme.zig`'s `Location`/`open`): `theme = NAME` loads `NAME` as a
//! *baseline* that the user's own explicit `background`/`foreground`/
//! `cursor-color`/`palette` settings then override on a per-field (and,
//! for `palette`, per-index) basis -- upstream's own doc comment: "Any
//! additional colors specified via background, foreground, palette, etc.
//! will override the colors specified in the theme." This holds regardless
//! of where in the user's files `theme = NAME` appears, because upstream
//! loads the theme first and then *replays* the user's own already-parsed
//! settings on top (`Config.loadTheme`) -- so this port applies the same
//! "theme colors merged under -- not overwritten *by* line order to --  the
//! user's own explicit colors" rule (see `merge_colors`) rather than
//! treating `theme` as just another line in load order.
//!
//! `NAME` resolves (`theme.zig`'s `Location`/`open`) to an absolute path
//! directly, or else is searched for (no path separators allowed) in: (1)
//! `$XDG_CONFIG_HOME/ghostty/themes` (or `~/.config/ghostty/themes`), then
//! (2) macOS only, best-effort: `/Applications/Ghostty.app/Contents/
//! Resources/ghostty/themes` (a hardcoded guess at the install location --
//! see "Known limitations" in the crate README -- rather than a real
//! bundle/LaunchServices lookup). A theme file is just another Ghostty
//! config file (verified against the real bundled themes, which are all
//! plain `key = value` lines); this port reads only the same color keys it
//! reads from the user's own files, silently ignoring `theme`/`config-file`
//! if present, matching upstream's documented restriction that a theme file
//! cannot set either.
//!
//! **Scope limitation (light/dark theme switching): out of scope.** Ghostty
//! supports `theme = light:NAME,dark:NAME` to pick a different theme based
//! on the desktop appearance (`Theme.parseCLI`). This port only ever uses
//! the **light** side (`parse_theme_value` extracts it and discards `dark`)
//! -- there is no appearance-switching logic here at all, so a config using
//! only the light/dark form with a light-mode theme unsuited to a dark
//! window will look wrong until this is revisited. TODO(follow-up wave):
//! track the effective appearance and pick light vs. dark accordingly.
//!
//! ## Testability
//!
//! Everything below `load_user_font_config`/`load_user_color_config` is pure
//! with respect to the environment: discovery takes `home`/`xdg_config_home`
//! (and, for colors, the theme resources directory) as parameters, and
//! loading takes explicit root paths, so the unit tests run entirely on
//! fixture trees under `fixtures/ghostty_config/` -- no test reads `$HOME`,
//! `/Applications`, or the real user's config.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use labolabo_term::{ColorScheme, Rgb};

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

/// Parse a color value: the supported subset of `terminal.color.RGB.parse`
/// (see the module doc comment's "Color value syntax") -- `#rgb`/`#rrggbb`
/// and the same two forms without the leading `#`. `None` means the value
/// isn't in a supported form (or isn't valid hex); callers report this and
/// leave any previous value untouched, mirroring how an unparseable
/// `font-size` is handled.
fn parse_color(value: &str) -> Option<Rgb> {
    let input = value.trim_matches(WHITESPACE);
    let hex = input.strip_prefix('#').unwrap_or(input);
    match hex.len() {
        // One hex digit per channel, scaled 4-bit -> 8-bit (`* 0xFF / 0xF`,
        // i.e. `* 17`) -- `RGB.fromHex`'s `len == 1` case.
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
            Some(Rgb::new(r * 17, g * 17, b * 17))
        }
        // Two hex digits per channel -- the literal byte value.
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Rgb::new(r, g, b))
        }
        _ => None,
    }
}

/// Parse a `palette = N=COLOR` value's index per
/// `terminal.color.parsePaletteEntry` (via Zig's base-0 `parseInt`, which
/// Rust's `from_str_radix` doesn't auto-sniff): a bare decimal number, or
/// one of the (lowercase) `0x`/`0o`/`0b` prefixes for hex/octal/binary.
fn parse_palette_index(value: &str) -> Option<u8> {
    let (radix, digits) = if let Some(d) = value.strip_prefix("0x") {
        (16, d)
    } else if let Some(d) = value.strip_prefix("0o") {
        (8, d)
    } else if let Some(d) = value.strip_prefix("0b") {
        (2, d)
    } else {
        (10, value)
    };
    u8::from_str_radix(digits, radix).ok()
}

/// Parse a `palette = N=COLOR` value per
/// `terminal.color.parsePaletteEntry`: split at the first `=`, then parse
/// each side (whitespace around both is trimmed).
fn parse_palette_entry(value: &str) -> Option<(u8, Rgb)> {
    let (index, color) = value.split_once('=')?;
    let index = parse_palette_index(index.trim_matches(WHITESPACE))?;
    let color = parse_color(color.trim_matches(WHITESPACE))?;
    Some((index, color))
}

/// Parse a `theme = ...` value into the theme name to resolve (see the
/// module doc comment's "Scope limitation" -- only the *light* side of a
/// `light:NAME,dark:NAME` pair is ever used). Mirrors `Theme.parseCLI`'s
/// dispatch: no `,`/`=`/`:` at all means a bare name (used for both
/// appearances upstream, so no special-casing is needed here); otherwise
/// it's the light/dark form, and only the `light:` component is extracted.
fn parse_theme_value(value: &str) -> Option<String> {
    let trimmed = value.trim_matches(WHITESPACE);
    if trimmed.is_empty() {
        return None;
    }
    if !trimmed.contains(',') && !trimmed.contains('=') && !trimmed.contains(':') {
        return Some(trimmed.to_string());
    }
    for part in trimmed.split(',') {
        let part = part.trim_matches(WHITESPACE);
        if let Some(name) = part.strip_prefix("light:") {
            let name = name.trim_matches(WHITESPACE);
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    eprintln!(
        "labolabo-app: ghostty theme {value:?} uses light/dark syntax without a usable \
         \"light:\" side; ignoring (dark-mode theme selection is out of scope)"
    );
    None
}

/// Apply one config line to `colors` if its key is one of the four color
/// keys (`background`/`foreground`/`cursor-color`/`palette`); a no-op for
/// any other key. Shared between the main `Loader` (the user's own files)
/// and `load_theme_colors` (a theme file) since both read exactly these
/// keys the same way -- see the module doc comment's "Theme resolution".
fn apply_color_line(colors: &mut ColorScheme, key: &str, value: Option<&str>) {
    match key {
        "background" => set_scalar_color(&mut colors.background, value, "background"),
        "foreground" => set_scalar_color(&mut colors.foreground, value, "foreground"),
        "cursor-color" => set_scalar_color(&mut colors.cursor, value, "cursor-color"),
        "palette" => {
            if let Some(v) = value {
                match parse_palette_entry(v) {
                    Some(entry) => colors.palette.push(entry),
                    None => eprintln!(
                        "labolabo-app: ignoring unparseable palette entry {v:?} in ghostty config"
                    ),
                }
            }
        }
        _ => {}
    }
}

/// Parse-and-set one scalar color key's value, matching `font-size`'s
/// "last valid value wins, unparseable is reported and ignored" handling
/// (a line with no `=` is a no-op, same as any other key here).
fn set_scalar_color(slot: &mut Option<Rgb>, value: Option<&str>, key: &str) {
    if let Some(v) = value {
        match parse_color(v) {
            Some(c) => *slot = Some(c),
            None => eprintln!(
                "labolabo-app: ignoring unsupported color value {v:?} for {key} in ghostty config"
            ),
        }
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
///
/// `#[cfg_attr(target_os = "windows", allow(dead_code))]`: only
/// `load_user_font_config`/`load_user_color_config`'s non-Windows arm calls
/// this (Windows uses [`windows_config_paths`] instead) -- same
/// dead-code-avoidance idiom `ide_open.rs` uses for its macOS-only helpers,
/// needed to keep `-D warnings` green on the `rust-app-windows` CI job.
#[cfg_attr(target_os = "windows", allow(dead_code))]
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

/// The Windows counterpart of [`default_config_paths`]: `%APPDATA%\ghostty\
/// config` then `\config.ghostty` (later wins, same two-filename convention
/// as the unix XDG search).
///
/// **Best-effort, unverified against a real upstream Windows Ghostty**:
/// Ghostty itself does not ship an official Windows build as of this
/// writing (macOS/Linux only), so there is no documented "where does
/// Ghostty look for its config on Windows" spec to port faithfully the way
/// [`default_config_paths`] ports Ghostty's real `Config.loadDefaultFiles`
/// (see the module doc comment). `%APPDATA%` (`Library/Application Support`'s
/// Windows analog -- the per-user roaming-profile settings directory every
/// native Windows app uses) is a reasonable, XDG-equivalent guess for where
/// a hypothetical Windows Ghostty *would* put it, not a confirmed one; a
/// user who manually places a config file there (e.g. anticipating a future
/// official Windows build, or running Ghostty under WSL with a Windows-side
/// copy) gets it picked up, but this is unproven, not "the same faithful
/// port" claim the unix paths carry. Pure, so it's unit-tested the same way
/// as `default_config_paths` despite the lower confidence in its target
/// paths.
#[cfg(target_os = "windows")]
pub fn windows_config_paths(appdata: &Path) -> Vec<PathBuf> {
    let dir = appdata.join("ghostty");
    vec![dir.join("config"), dir.join("config.ghostty")]
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
    /// The user's own explicit color settings (`background`/`foreground`/
    /// `cursor-color`/`palette`) -- **not** merged with any theme yet; see
    /// `extract_color_config`/`merge_colors` for that.
    colors: ColorScheme,
    /// The last `theme = ...` value's resolved name (scalar, last-wins --
    /// same as `font-size`), if any. `extract_font_config` never reads this;
    /// it's populated as a harmless byproduct of sharing one file-traversal
    /// pass with `extract_color_config`.
    theme: Option<String>,
    /// The `config-file` include queue, in declaration order. Grows while
    /// includes are being processed (recursive includes append here).
    includes: Vec<(PathBuf, bool)>,
}

impl Loader {
    /// Parse one file's text: apply font/color keys, record `theme`, queue
    /// `config-file` values.
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
                "background" | "foreground" | "cursor-color" | "palette" => {
                    apply_color_line(&mut self.colors, line.key, line.value)
                }
                "theme" => {
                    if let Some(v) = line.value {
                        if let Some(name) = parse_theme_value(v) {
                            self.theme = Some(name);
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
/// -- see the module doc comment): the shared traversal both
/// `extract_font_config` and `extract_color_config` build on, so file
/// discovery/include semantics are implemented (and tested) exactly once.
/// Returns the user's own fonts, explicit colors (not yet merged with any
/// theme), and last-set `theme` name.
fn extract_config(roots: &[PathBuf], home: &Path) -> (FontConfig, ColorScheme, Option<String>) {
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

    (loader.fonts, loader.colors, loader.theme)
}

/// Load the given root config files (in order) and then their `config-file`
/// includes, returning the extracted font settings. See `extract_config`.
pub fn extract_font_config(roots: &[PathBuf], home: &Path) -> FontConfig {
    extract_config(roots, home).0
}

/// Read the real user's Ghostty font configuration: `$HOME` /
/// `$XDG_CONFIG_HOME` discovery, macOS Application Support included on
/// macOS, `%APPDATA%\ghostty` on Windows (see [`windows_config_paths`] --
/// best-effort, unverified). Missing config files (or no home directory
/// resolvable at all) just mean defaults.
pub fn load_user_font_config() -> FontConfig {
    #[cfg(target_os = "windows")]
    {
        let Some((roots, home)) = windows_roots_and_home() else {
            return FontConfig::default();
        };
        extract_font_config(&roots, &home)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return FontConfig::default();
        };
        let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        let roots = default_config_paths(&home, xdg.as_deref(), cfg!(target_os = "macos"));
        extract_font_config(&roots, &home)
    }
}

/// Windows-only: `%APPDATA%`-rooted config search paths plus a "home"
/// directory for `config-file` include resolution (relative-path/`~/`
/// handling in `extract_config`/`resolve_include_path` -- see the module
/// doc comment's "config-file includes"). `%USERPROFILE%` is the Windows
/// analog of unix `$HOME` for this purpose (falls back to `%APPDATA%`
/// itself, which is always some subdirectory of the real user profile, if
/// `USERPROFILE` is somehow unset -- keeps this a total function over "no
/// home directory resolvable at all" rather than two independently-missing
/// env vars each needing their own bail-out). `None` only when `%APPDATA%`
/// itself is unset, which does not happen on a real Windows session (every
/// interactive user has one) but could on a stripped-down CI/service
/// context -- same "just mean defaults" fallback the unix `$HOME`-unset
/// case already has.
#[cfg(target_os = "windows")]
fn windows_roots_and_home() -> Option<(Vec<PathBuf>, PathBuf)> {
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from)?;
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| appdata.clone());
    Some((windows_config_paths(&appdata), home))
}

// --- colors: theme resolution + merge --------------------------------------

/// Theme search directories in priority order (`theme.zig`'s `Location`
/// enum + `LocationIterator`): the user's own `themes` subdirectory first,
/// then (optionally -- macOS only in real usage, see `load_user_color_
/// config`) the Ghostty app bundle's own bundled themes.
fn theme_search_dirs(
    home: &Path,
    xdg_config_home: Option<&Path>,
    resources_themes_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let xdg_base = match xdg_config_home {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => home.join(".config"),
    };
    let mut dirs = vec![xdg_base.join("ghostty/themes")];
    if let Some(dir) = resources_themes_dir {
        dirs.push(dir.to_path_buf());
    }
    dirs
}

/// Resolve a `theme` name to a file path per `theme.zig`'s `open`: an
/// absolute path is used directly (existence is checked, matching upstream
/// -- an absolute theme that doesn't exist is *not* searched for elsewhere);
/// otherwise (no path separators allowed) each of `theme_search_dirs` is
/// tried in order for the first `name` that exists as a regular file.
fn resolve_theme_path(
    name: &str,
    home: &Path,
    xdg_config_home: Option<&Path>,
    resources_themes_dir: Option<&Path>,
) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.is_absolute() {
        return is_regular_file(candidate)
            .then(|| candidate.to_path_buf())
            .or_else(|| {
                eprintln!(
                "labolabo-app: ghostty theme {name:?}: not reading because it is not a regular file"
            );
                None
            });
    }
    if name.contains('/') || name.contains('\\') {
        eprintln!(
            "labolabo-app: ghostty theme {name:?} contains a path separator; only an absolute \
             path may -- ignoring theme"
        );
        return None;
    }
    for dir in theme_search_dirs(home, xdg_config_home, resources_themes_dir) {
        let path = dir.join(name);
        if is_regular_file(&path) {
            return Some(path);
        }
    }
    eprintln!("labolabo-app: ghostty theme {name:?} not found in any theme directory; ignoring");
    None
}

fn is_regular_file(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

/// Read a theme file's color settings. A theme file is just another Ghostty
/// config file (see the module doc comment's "Theme resolution"), but this
/// reads only the same color keys `Loader` does -- `theme`/`config-file`
/// (and anything else) are silently ignored if present, matching upstream's
/// documented restriction that a theme file cannot set either.
fn load_theme_colors(path: &Path) -> ColorScheme {
    let mut colors = ColorScheme::default();
    let Some(text) = read_config_text(path) else {
        eprintln!(
            "labolabo-app: failed to read ghostty theme file {}",
            path.display()
        );
        return colors;
    };
    for raw in text.lines() {
        let Some(line) = parse_line(raw) else {
            continue;
        };
        if matches!(
            line.key,
            "background" | "foreground" | "cursor-color" | "palette"
        ) {
            apply_color_line(&mut colors, line.key, line.value);
        }
    }
    colors
}

/// Merge a theme's colors under the user's own explicit settings: "Any
/// additional colors specified via background, foreground, palette, etc.
/// will override the colors specified in the theme" (`Config.zig`'s `theme`
/// doc comment). Scalar fields: the user's value wins if set. Palette: the
/// theme's entries are listed first, then the user's -- since a consumer
/// applies `ColorScheme::palette` in order with a later same-index entry
/// winning (`ColorScheme::apply_palette`), this reproduces per-index
/// override without a separate merge step.
fn merge_colors(theme: ColorScheme, user: ColorScheme) -> ColorScheme {
    let mut palette = theme.palette;
    palette.extend(user.palette);
    ColorScheme {
        foreground: user.foreground.or(theme.foreground),
        background: user.background.or(theme.background),
        cursor: user.cursor.or(theme.cursor),
        palette,
    }
}

/// Load the given root config files the same way `extract_font_config`
/// does, then resolve+merge any `theme` (see `merge_colors`). `home`/
/// `xdg_config_home` also drive theme search (see `theme_search_dirs`);
/// `resources_themes_dir` is the Ghostty app bundle's bundled-themes
/// directory, if any (real usage: macOS-only, see `load_user_color_config`;
/// tests inject a fixture directory or `None`).
pub fn extract_color_config(
    roots: &[PathBuf],
    home: &Path,
    xdg_config_home: Option<&Path>,
    resources_themes_dir: Option<&Path>,
) -> ColorScheme {
    let (_, user_colors, theme_name) = extract_config(roots, home);

    let theme_colors = theme_name
        .and_then(|name| resolve_theme_path(&name, home, xdg_config_home, resources_themes_dir))
        .map(|path| load_theme_colors(&path))
        .unwrap_or_default();

    merge_colors(theme_colors, user_colors)
}

/// Read the real user's Ghostty color configuration (`background`/
/// `foreground`/`cursor-color`/`palette`, plus `theme` resolution): same
/// file discovery as `load_user_font_config`, plus a best-effort macOS
/// resources-dir guess for themes (see the module doc comment's "Theme
/// resolution"). Missing config/theme files (or no home directory resolvable
/// at all) just mean `ColorScheme::default()` -- every backend's own
/// built-in colors. No bundled-themes-directory guess on Windows (no app
/// bundle concept to guess a path inside of) -- a Windows user's `theme = `
/// only ever resolves against their own `ghostty/themes` subdirectory.
pub fn load_user_color_config() -> ColorScheme {
    #[cfg(target_os = "windows")]
    {
        let Some((roots, home)) = windows_roots_and_home() else {
            return ColorScheme::default();
        };
        extract_color_config(&roots, &home, None, None)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return ColorScheme::default();
        };
        let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        let roots = default_config_paths(&home, xdg.as_deref(), cfg!(target_os = "macos"));
        let resources_themes_dir = cfg!(target_os = "macos")
            .then(|| PathBuf::from("/Applications/Ghostty.app/Contents/Resources/ghostty/themes"));
        extract_color_config(
            &roots,
            &home,
            xdg.as_deref(),
            resources_themes_dir.as_deref(),
        )
    }
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

    // --- windows_config_paths (Windows only -- see its doc comment) --------

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_paths_are_rooted_at_appdata_ghostty() {
        let paths = windows_config_paths(Path::new(r"C:\Users\u\AppData\Roaming"));
        assert_eq!(
            paths,
            vec![
                PathBuf::from(r"C:\Users\u\AppData\Roaming\ghostty\config"),
                PathBuf::from(r"C:\Users\u\AppData\Roaming\ghostty\config.ghostty"),
            ]
        );
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

    // --- parse_color ---------------------------------------------------------

    #[test]
    fn parse_color_accepts_hashed_and_bare_hex_short_and_long_forms() {
        assert_eq!(parse_color("#000000"), Some(Rgb::new(0, 0, 0)));
        assert_eq!(parse_color("#0A0B0C"), Some(Rgb::new(10, 11, 12)));
        assert_eq!(parse_color("0A0B0C"), Some(Rgb::new(10, 11, 12)));
        assert_eq!(parse_color("FFFFFF"), Some(Rgb::new(255, 255, 255)));
        // Real-world case: a user theme file observed in the wild uses bare
        // (no `#`) hex, exactly this form.
        assert_eq!(parse_color("1A202C"), Some(Rgb::new(0x1A, 0x20, 0x2C)));
        // 3-digit short form: each nibble is scaled 4-bit -> 8-bit (`* 17`).
        assert_eq!(parse_color("FFF"), Some(Rgb::new(255, 255, 255)));
        assert_eq!(parse_color("#345"), Some(Rgb::new(0x33, 0x44, 0x55)));
        // Leading/trailing whitespace is trimmed.
        assert_eq!(
            parse_color("  #AABBCC   "),
            Some(Rgb::new(0xAA, 0xBB, 0xCC))
        );
    }

    #[test]
    fn parse_color_rejects_unsupported_forms() {
        // X11 named colors: unsupported (out of scope for this wave).
        assert_eq!(parse_color("black"), None);
        // `rgb:`/`rgbi:` device syntax: unsupported.
        assert_eq!(parse_color("rgb:12/34/56"), None);
        assert_eq!(parse_color("rgbi:0.1/0.2/0.3"), None);
        // 12-/16-bit-per-channel forms: unsupported.
        assert_eq!(parse_color("#123456789"), None);
        // Invalid hex digits, empty, and wrong lengths.
        assert_eq!(parse_color("#GGGGGG"), None);
        assert_eq!(parse_color(""), None);
        assert_eq!(parse_color("#1234"), None);
    }

    // --- parse_palette_index / parse_palette_entry ----------------------------

    #[test]
    fn palette_index_parses_decimal_and_prefixed_radixes() {
        assert_eq!(parse_palette_index("5"), Some(5));
        assert_eq!(parse_palette_index("0x0F"), Some(15));
        assert_eq!(parse_palette_index("0o17"), Some(15));
        assert_eq!(parse_palette_index("0b1111"), Some(15));
        // Out of u8 range -> None (reported and skipped by the caller).
        assert_eq!(parse_palette_index("256"), None);
        assert_eq!(parse_palette_index("not-a-number"), None);
    }

    #[test]
    fn palette_entry_splits_at_first_equals_and_trims_whitespace() {
        assert_eq!(
            parse_palette_entry("0=#AABBCC"),
            Some((0, Rgb::new(0xAA, 0xBB, 0xCC)))
        );
        assert_eq!(
            parse_palette_entry(" 1 = #DDEEFF "),
            Some((1, Rgb::new(0xDD, 0xEE, 0xFF)))
        );
        assert_eq!(parse_palette_entry("no-equals-sign"), None);
        assert_eq!(parse_palette_entry("1=notacolor"), None);
        assert_eq!(parse_palette_entry("999=#ffffff"), None);
    }

    // --- parse_theme_value -----------------------------------------------------

    #[test]
    fn theme_value_bare_name_is_used_for_both_appearances() {
        assert_eq!(
            parse_theme_value("catppuccin-mocha"),
            Some("catppuccin-mocha".to_string())
        );
        assert_eq!(parse_theme_value("  spaced  "), Some("spaced".to_string()));
        assert_eq!(parse_theme_value(""), None);
    }

    #[test]
    fn theme_value_light_dark_pair_keeps_only_the_light_side() {
        assert_eq!(
            parse_theme_value("light:foo,dark:bar"),
            Some("foo".to_string())
        );
        // Whitespace around parts, and dark-before-light order.
        assert_eq!(
            parse_theme_value(" dark : bar , light:foo "),
            Some("foo".to_string())
        );
    }

    // --- extract_color_config: basic parsing ------------------------------------

    #[test]
    fn colors_basic_fixture_parses_scalars_and_palette_with_warn_and_skip() {
        let root = fixture_root().join("colors_basic/config");
        // No theme resolves ("catppuccin-mocha" isn't in this fixture tree),
        // so the result is exactly the user's own explicit colors.
        let colors = extract_color_config(&[root], &fixture_root(), None, None);
        assert_eq!(colors.background, Some(Rgb::new(0x10, 0x12, 0x14)));
        assert_eq!(colors.foreground, Some(Rgb::new(0x1A, 0x2B, 0x3C)));
        assert_eq!(colors.cursor, Some(Rgb::new(0xFF, 0x00, 0xFF)));
        let resolved = colors.apply_palette([Rgb::BLACK; 256]);
        assert_eq!(resolved[0], Rgb::new(0, 0, 0));
        assert_eq!(resolved[1], Rgb::new(0xAA, 0xBB, 0xCC));
        assert_eq!(resolved[2], Rgb::new(0x22, 0x33, 0x44));
        // The invalid-index and invalid-color palette lines contributed
        // nothing; every other index keeps the base table's color.
        assert_eq!(resolved[3], Rgb::BLACK);
    }

    #[test]
    fn nonexistent_and_empty_roots_yield_default_color_scheme() {
        let missing = fixture_root().join("does-not-exist/config");
        let empty = fixture_root().join("empty/config");
        let colors = extract_color_config(&[missing, empty], &fixture_root(), None, None);
        assert_eq!(colors, ColorScheme::default());
    }

    // --- extract_color_config: theme resolution + merge -------------------------

    #[test]
    fn theme_colors_are_merged_under_the_users_own_explicit_colors() {
        let xdg = fixture_root().join("colors_theme/xdg");
        let root = xdg.join("ghostty/config");
        let colors = extract_color_config(&[root], &fixture_root(), Some(&xdg), None);

        // background/cursor: not set by the user -> theme's value.
        assert_eq!(colors.background, Some(Rgb::new(0x11, 0x11, 0x11)));
        assert_eq!(colors.cursor, Some(Rgb::new(0xDD, 0xDD, 0xDD)));
        // foreground: the user's own value wins over the theme's.
        assert_eq!(colors.foreground, Some(Rgb::new(0xCC, 0xCC, 0xCC)));
        // palette: index 1 is set by both -- the user's wins; index 2 is
        // theme-only and comes through untouched.
        let resolved = colors.apply_palette([Rgb::BLACK; 256]);
        assert_eq!(resolved[1], Rgb::new(0x00, 0xFF, 0x00));
        assert_eq!(resolved[2], Rgb::new(0x00, 0x00, 0xFF));
    }

    #[test]
    fn unresolvable_theme_is_logged_and_falls_back_to_user_colors_only() {
        let root = fixture_root().join("colors_theme_missing/config");
        let colors = extract_color_config(&[root], &fixture_root(), None, None);
        assert_eq!(colors.background, None);
        assert_eq!(colors.foreground, Some(Rgb::new(0xAB, 0xCD, 0xEF)));
    }

    #[test]
    fn light_dark_theme_syntax_uses_only_the_light_theme_file() {
        // No "darktheme" file exists in this fixture tree at all -- if the
        // implementation ever tried to resolve the dark side, this would
        // fail loudly (a theme-not-found warning, and no colors set).
        let xdg = fixture_root().join("colors_theme_light_dark/xdg");
        let root = xdg.join("ghostty/config");
        let colors = extract_color_config(&[root], &fixture_root(), Some(&xdg), None);
        assert_eq!(colors.background, Some(Rgb::new(0xFF, 0xFF, 0xFF)));
        assert_eq!(colors.foreground, Some(Rgb::new(0x00, 0x00, 0x00)));
    }

    #[test]
    fn theme_falls_back_to_the_resources_directory_when_not_in_the_user_themes_dir() {
        let xdg = fixture_root().join("colors_theme_resources/xdg");
        let resources = fixture_root().join("colors_theme_resources/resources/themes");
        let root = xdg.join("ghostty/config");
        let colors = extract_color_config(&[root], &fixture_root(), Some(&xdg), Some(&resources));
        assert_eq!(colors.background, Some(Rgb::new(0x20, 0x20, 0x20)));
    }

    // --- resolve_theme_path ------------------------------------------------------

    #[test]
    fn resolve_theme_path_accepts_an_existing_absolute_path_directly() {
        let path = fixture_root().join("colors_theme_absolute/external_theme");
        let resolved = resolve_theme_path(
            path.to_str().unwrap(),
            Path::new("/nonexistent"),
            None,
            None,
        );
        assert_eq!(resolved, Some(path));
    }

    #[test]
    fn resolve_theme_path_rejects_relative_names_with_path_separators() {
        let resolved = resolve_theme_path("sub/theme", &fixture_root(), None, None);
        assert_eq!(resolved, None);
    }
}
