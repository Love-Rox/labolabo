//! i18n quality gates (wave 6f) -- three source-level checks over
//! `locales/{ja,en}.yml` and `src/**/*.rs`:
//!
//! 1. **Key parity**: every key exists in *both* locale files, with a
//!    non-empty value (the task brief's "全キーが両言語に存在するテスト").
//!    `rust_i18n`'s own `fallback = "en"` would paper over a missing ja key
//!    at runtime -- this test is what actually blocks it.
//! 2. **Used-key existence**: every string-literal key passed to `t!()`
//!    anywhere in `src/` exists in the locale tables -- a typo'd key would
//!    otherwise silently render as the raw key string at runtime (rust-i18n
//!    has no compile-time key check).
//! 3. **No hardcoded Japanese UI strings** (the brief's 取りこぼしゲート):
//!    no *string literal* in production code (comments and `#[cfg(test)]`
//!    modules excluded) may contain Japanese characters -- everything must
//!    go through the locale tables. Implemented as a small string-literal
//!    scanner rather than a plain `grep` so Japanese in doc/line comments
//!    (pervasive and fine in this codebase) doesn't false-positive.
//!
//! These read the crate's *sources* via `CARGO_MANIFEST_DIR`, not the
//! compiled binary -- the locale YAML here is the same file
//! `rust_i18n::i18n!` embeds at compile time, so testing the file *is*
//! testing what ships.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// MARK: - locale file loading

/// Flattens a `_version: 1` locale YAML mapping into dotted keys
/// (`menu.file.new_tab`, ...), asserting every leaf is a non-empty string.
fn flatten_locale(file_name: &str) -> BTreeSet<String> {
    let path = manifest_dir().join("locales").join(file_name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {path:?}: {err}"));
    let root: serde_yaml::Value =
        serde_yaml::from_str(&text).unwrap_or_else(|err| panic!("failed to parse {path:?}: {err}"));
    let mut keys = BTreeSet::new();
    flatten_into(&root, String::new(), file_name, &mut keys);
    keys
}

fn flatten_into(
    value: &serde_yaml::Value,
    prefix: String,
    file_name: &str,
    keys: &mut BTreeSet<String>,
) {
    let serde_yaml::Value::Mapping(map) = value else {
        panic!("{file_name}: expected a mapping at {prefix:?}");
    };
    for (k, v) in map {
        let k = k
            .as_str()
            .unwrap_or_else(|| panic!("{file_name}: non-string key under {prefix:?}"));
        if k == "_version" {
            continue;
        }
        let full = if prefix.is_empty() {
            k.to_string()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            serde_yaml::Value::Mapping(_) => flatten_into(v, full, file_name, keys),
            serde_yaml::Value::String(s) => {
                assert!(
                    !s.trim().is_empty(),
                    "{file_name}: key {full} has an empty value"
                );
                keys.insert(full);
            }
            other => panic!("{file_name}: key {full} has a non-string value: {other:?}"),
        }
    }
}

#[test]
fn ja_and_en_locale_files_define_exactly_the_same_keys() {
    let ja = flatten_locale("ja.yml");
    let en = flatten_locale("en.yml");
    let only_ja: Vec<_> = ja.difference(&en).collect();
    let only_en: Vec<_> = en.difference(&ja).collect();
    assert!(
        only_ja.is_empty() && only_en.is_empty(),
        "locale key sets differ -- only in ja.yml: {only_ja:?}; only in en.yml: {only_en:?}"
    );
    assert!(!ja.is_empty(), "locale files define no keys at all");
}

// MARK: - source scanning helpers

fn production_rust_sources() -> Vec<PathBuf> {
    let src = manifest_dir().join("src");
    let mut files = Vec::new();
    collect_rs_files(&src, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "found no .rs files under {src:?} -- scanner is miswired"
    );
    files
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Reads a source file with its `#[cfg(test)]` region removed. Heuristic:
/// this codebase's convention (every file in `src/`) puts the test module
/// last, so everything from the first `#[cfg(test)]` to EOF is test code.
fn production_text(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap();
    match text.find("#[cfg(test)]") {
        Some(idx) => text[..idx].to_string(),
        None => text,
    }
}

/// Extracts every string literal (normal and raw) from `text`, skipping
/// `//` line comments and `/* */` block comments. Returns `(line, content)`
/// pairs. A tiny purpose-built scanner, not a Rust parser -- good enough
/// for the two uses below (both operate on literal *contents* only).
fn string_literals(text: &str) -> Vec<(usize, String)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = text[i + 2..].find("*/").map(|j| i + 2 + j + 2);
                let stop = end.unwrap_or(bytes.len());
                line += text[i..stop].matches('\n').count();
                i = stop;
            }
            b'r' if matches!(bytes.get(i + 1), Some(b'"') | Some(b'#')) => {
                // possible raw string literal r"..." / r#"..."#
                let mut j = i + 1;
                let mut hashes = 0;
                while bytes.get(j) == Some(&b'#') {
                    hashes += 1;
                    j += 1;
                }
                if bytes.get(j) == Some(&b'"') {
                    let close = format!("\"{}", "#".repeat(hashes));
                    let content_start = j + 1;
                    let end = text[content_start..]
                        .find(&close)
                        .map(|k| content_start + k)
                        .unwrap_or(bytes.len());
                    let content = &text[content_start..end];
                    out.push((line, content.to_string()));
                    line += text[i..end].matches('\n').count();
                    i = end + close.len();
                } else {
                    i += 1;
                }
            }
            b'"' => {
                let mut j = i + 1;
                let mut content = String::new();
                while j < bytes.len() {
                    match bytes[j] {
                        b'\\' => {
                            // keep escapes opaque -- \u{...} etc. never
                            // contain literal Japanese
                            j += 2;
                        }
                        b'"' => break,
                        _ => {
                            let ch_start = j;
                            let mut ch_end = j + 1;
                            while ch_end < bytes.len() && (bytes[ch_end] & 0xC0) == 0x80 {
                                ch_end += 1;
                            }
                            content.push_str(&text[ch_start..ch_end]);
                            j = ch_end;
                        }
                    }
                }
                line += content.matches('\n').count();
                out.push((line, content));
                i = j + 1;
            }
            b'\'' => {
                // char literal ('a', '\n', '\u{1F600}') vs. lifetime ('static).
                // A char literal always closes with a ' within a few bytes;
                // a lifetime never does before a non-identifier char.
                if bytes.get(i + 1) == Some(&b'\\') {
                    let end = text[i + 2..].find('\'').map(|j| i + 2 + j + 1);
                    i = end.unwrap_or(i + 1);
                } else if bytes.get(i + 2) == Some(&b'\'') {
                    i += 3;
                } else {
                    i += 1; // lifetime
                }
            }
            _ => i += 1,
        }
    }
    out
}

fn contains_japanese(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c,
            '\u{3041}'..='\u{3096}' // hiragana
            | '\u{30A1}'..='\u{30FA}' // katakana
            | '\u{4E00}'..='\u{9FA0}' // CJK unified ideographs (一-龠)
        )
    })
}

#[test]
fn production_code_has_no_hardcoded_japanese_string_literals() {
    let mut offenders = Vec::new();
    for path in production_rust_sources() {
        let text = production_text(&path);
        for (line, literal) in string_literals(&text) {
            if contains_japanese(&literal) {
                offenders.push(format!(
                    "{}:{line}: {:?}",
                    path.strip_prefix(manifest_dir()).unwrap().display(),
                    literal
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "hardcoded Japanese UI strings found (use t!() + locales/*.yml instead):\n{}",
        offenders.join("\n")
    );
}

// MARK: - t!() key existence

/// Every string-literal key passed to `t!(` in production code. Handles the
/// rustfmt-split multi-line form (`t!(\n    "key",`) by skipping whitespace
/// between `t!(` and the opening quote.
fn used_t_keys() -> BTreeSet<(String, String)> {
    let mut used = BTreeSet::new();
    for path in production_rust_sources() {
        let text = production_text(&path);
        let mut rest = text.as_str();
        let mut offset = 0;
        while let Some(pos) = rest.find("t!(") {
            // `format!(`/`assert!(`-style macros also end in `t!(` -- only a
            // bare `t` (no identifier char before it) is the i18n macro.
            let abs = offset + pos;
            let preceded_by_ident = text[..abs]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            let after = &rest[pos + 3..];
            if !preceded_by_ident {
                let trimmed = after.trim_start();
                if let Some(stripped) = trimmed.strip_prefix('"') {
                    if let Some(end) = stripped.find('"') {
                        used.insert((
                            path.strip_prefix(manifest_dir())
                                .unwrap()
                                .display()
                                .to_string(),
                            stripped[..end].to_string(),
                        ));
                    }
                }
            }
            offset = abs + 3;
            rest = after;
        }
    }
    used
}

#[test]
fn every_t_macro_key_used_in_source_exists_in_both_locales() {
    let ja = flatten_locale("ja.yml");
    let en = flatten_locale("en.yml");
    let used = used_t_keys();
    assert!(
        !used.is_empty(),
        "found no t!(..) call sites -- scanner is miswired"
    );
    let mut missing = Vec::new();
    for (file, key) in &used {
        if !ja.contains(key) || !en.contains(key) {
            missing.push(format!("{file}: t!({key:?}) has no locale entry"));
        }
    }
    assert!(
        missing.is_empty(),
        "t!() keys missing from locales/*.yml:\n{}",
        missing.join("\n")
    );
}
