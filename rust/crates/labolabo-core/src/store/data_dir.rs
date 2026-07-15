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
pub fn rust_app_data_dir() -> PathBuf {
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
}
