//! Faithful port of `Sources/LaboLaboStore/AppDataDirectory.swift`.
//!
//! Resolves the root directory this app stores its persisted data under.
//! Swift only implements macOS (`~/Library/Application Support/LaboLabo`);
//! its doc comment sketches Windows/Linux as future work. This port
//! implements all three today (the Rust core is cross-platform from day
//! one — see `rust/README.md`'s CI matrix), each behind a `cfg` branch:
//!
//! - macOS: `~/Library/Application Support/LaboLabo` (byte-identical to the
//!   Swift implementation, which resolves
//!   `FileManager.default.urls(for: .applicationSupportDirectory, in:
//!   .userDomainMask)[0]` — always `~/Library/Application Support` for the
//!   per-user domain on macOS — and appends `LaboLabo`).
//! - Linux: `$XDG_DATA_HOME/LaboLabo`, falling back to
//!   `~/.local/share/LaboLabo` when `XDG_DATA_HOME` is unset or empty (the
//!   XDG Base Directory spec's documented default), exactly as the Swift
//!   doc comment sketches.
//! - Windows: `%APPDATA%\LaboLabo`, as the Swift doc comment sketches.
//!
//! No golden coverage: there is nothing to compare against (macOS is the
//! only platform the Swift side runs on), and the path is a pure function
//! of the platform + a couple of environment variables / `$HOME`, not a
//! parser ported from a Swift algorithm.

use std::path::PathBuf;

/// `~/Library/Application Support/LaboLabo` on macOS,
/// `$XDG_DATA_HOME/LaboLabo` (falling back to `~/.local/share/LaboLabo`) on
/// Linux, `%APPDATA%\LaboLabo` on Windows.
pub fn app_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home_dir()
            .join("Library")
            .join("Application Support")
            .join("LaboLabo")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| home_dir().join(".local").join("share"));
        base.join("LaboLabo")
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home_dir);
        base.join("LaboLabo")
    }
}

/// Same per-OS base directory logic as [`app_data_dir`], but under a
/// distinct `"LaboLabo-rs"` leaf — the Rust port's *own* data directory,
/// deliberately never colliding with the Swift app's `app_data_dir()`
/// (`.../LaboLabo`). Both apps can independently write into their own tree
/// (e.g. running the Swift app and this Rust port's `labolabo-app` side by
/// side) without a chance of two processes touching the same SQLite file —
/// see `store::task_database`'s module doc comment for why that matters
/// (the Rust port's `Task` schema is not GRDB-compatible and must never be
/// opened by, or written by, the Swift `SessionDatabase`).
///
/// **`LABOLABO_RS_DATA_DIR` override (developer escape hatch)**: when this
/// environment variable is set and non-empty, its value is used verbatim as
/// the data directory, bypassing the per-OS default entirely. Purpose: a
/// development-time smoke run (`LABOLABO_RS_DATA_DIR=$(mktemp -d) cargo run
/// -p labolabo-app`) must not touch the real user database — restoring real
/// Tasks means spawning shells in (and injecting Claude hooks into) those
/// Tasks' real working directories, which an exploratory run has no
/// business doing. An empty value is treated as unset (same rule as the
/// `XDG_DATA_HOME` handling below and `ghostty_config`'s `XDG_CONFIG_HOME`).
pub fn rust_app_data_dir() -> PathBuf {
    rust_app_data_dir_from(std::env::var_os("LABOLABO_RS_DATA_DIR").as_deref())
}

/// The env-value-as-parameter core of [`rust_app_data_dir`], split out so
/// the override rule is unit-testable without mutating the real process
/// environment (mutating env in tests races other tests on the same
/// process; this crate's convention — same as `ghostty_config`'s
/// `default_config_paths` XDG tests in `labolabo-app` — is to keep env
/// reads in a thin caller and test the pure function).
fn rust_app_data_dir_from(override_dir: Option<&std::ffi::OsStr>) -> PathBuf {
    if let Some(dir) = override_dir {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    #[cfg(target_os = "macos")]
    {
        home_dir()
            .join("Library")
            .join("Application Support")
            .join("LaboLabo-rs")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| home_dir().join(".local").join("share"));
        base.join("LaboLabo-rs")
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home_dir);
        base.join("LaboLabo-rs")
    }
}

#[cfg(unix)]
fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_with_labolabo() {
        assert_eq!(app_data_dir().file_name().unwrap(), "LaboLabo");
    }

    #[test]
    fn rust_app_data_dir_never_collides_with_swift_app_data_dir() {
        assert_ne!(rust_app_data_dir(), app_data_dir());
        assert_eq!(rust_app_data_dir().file_name().unwrap(), "LaboLabo-rs");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_matches_swift_app_data_directory() {
        let expected = home_dir()
            .join("Library")
            .join("Application Support")
            .join("LaboLabo");
        assert_eq!(app_data_dir(), expected);
    }

    // --- LABOLABO_RS_DATA_DIR override ------------------------------------
    //
    // Tested through the pure `rust_app_data_dir_from` (env value as a
    // parameter), never by mutating the real process environment -- see
    // that function's doc comment.

    #[test]
    fn data_dir_override_is_used_verbatim_when_set() {
        assert_eq!(
            rust_app_data_dir_from(Some(std::ffi::OsStr::new("/tmp/labolabo-scratch"))),
            PathBuf::from("/tmp/labolabo-scratch")
        );
    }

    #[test]
    fn empty_data_dir_override_is_treated_as_unset() {
        assert_eq!(
            rust_app_data_dir_from(Some(std::ffi::OsStr::new(""))),
            rust_app_data_dir_from(None)
        );
    }

    #[test]
    fn absent_data_dir_override_falls_back_to_the_per_os_default() {
        assert_eq!(
            rust_app_data_dir_from(None).file_name().unwrap(),
            "LaboLabo-rs"
        );
    }
}
