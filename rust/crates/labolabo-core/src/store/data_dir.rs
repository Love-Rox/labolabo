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
//!
//! ## The 1.1.0 rename: `rust_app_data_dir` now equals `app_data_dir`
//!
//! Before the Rust port's 1.1.0 "LaboLabo-rs" → "LaboLabo" rename (the
//! Swift app's own retirement made the name free), [`rust_app_data_dir`]
//! resolved to a *different* leaf directory than [`app_data_dir`]
//! (`.../LaboLabo-rs` vs `.../LaboLabo`) purely to avoid two apps' database
//! files colliding. As of 1.1.0 the two functions resolve to the **same**
//! directory — the two database files still never collide, because they
//! have different *filenames* (`labolabo.db` for the Swift app's
//! `SessionDatabase`, `tasks.db` for this port's own `TaskDatabase` — see
//! `store::task_database`'s module doc comment), so sharing the containing
//! directory is safe.
//!
//! [`migrate_legacy_rust_data_dir`] is the one-time startup migration that
//! moves an existing pre-1.1.0 installation's `tasks.db` from the old
//! `.../LaboLabo-rs/` directory into the new shared `.../LaboLabo/`
//! directory; see its own doc comment for the full contract.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Filename of this port's own SQLite database, under either
/// [`rust_app_data_dir`] (current) or the pre-1.1.0 legacy directory
/// (`legacy_rust_app_data_dir_from`) — named here (not just in
/// `store::task_database::TaskDatabase::default_path`, which must resolve
/// to the same value) so [`migrate_legacy_rust_data_dir`] doesn't need a
/// dependency on that module for one literal both places must agree on.
pub(super) const TASK_DB_FILE_NAME: &str = "tasks.db";

/// `~/Library/Application Support/LaboLabo` on macOS,
/// `$XDG_DATA_HOME/LaboLabo` (falling back to `~/.local/share/LaboLabo`) on
/// Linux, `%APPDATA%\LaboLabo` on Windows.
pub fn app_data_dir() -> PathBuf {
    platform_leaf_dir("LaboLabo")
}

/// Same per-OS base directory logic as [`app_data_dir`], parameterized by
/// the leaf directory name — shared by [`app_data_dir`],
/// [`rust_app_data_dir_from`]'s per-OS-default branch, and
/// `legacy_rust_app_data_dir_from`'s pre-1.1.0 branch, which differ only in
/// that leaf.
fn platform_leaf_dir(leaf: &str) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home_dir()
            .join("Library")
            .join("Application Support")
            .join(leaf)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| home_dir().join(".local").join("share"));
        base.join(leaf)
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home_dir);
        base.join(leaf)
    }
}

/// As of the 1.1.0 rename, the **same** per-OS directory as [`app_data_dir`]
/// (see this module's doc comment for why that's safe — different
/// filenames, not different directories, keep the two apps' database files
/// from colliding).
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
/// The env var itself keeps its pre-rename `_RS_` name for compatibility
/// with existing scripts/muscle memory — see `rust/README.md`'s smoke-run
/// section for the note on why it wasn't renamed alongside the app.
pub fn rust_app_data_dir() -> PathBuf {
    rust_app_data_dir_from(std::env::var_os("LABOLABO_RS_DATA_DIR").as_deref())
}

/// The env-value-as-parameter core of [`rust_app_data_dir`], split out so
/// the override rule is unit-testable without mutating the real process
/// environment (mutating env in tests races other tests on the same
/// process; this crate's convention — same as `ghostty_config`'s
/// `default_config_paths` XDG tests in `labolabo-app` — is to keep env
/// reads in a thin caller and test the pure function).
fn rust_app_data_dir_from(override_dir: Option<&OsStr>) -> PathBuf {
    if let Some(dir) = override_dir {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    platform_leaf_dir("LaboLabo")
}

/// The pre-1.1.0 per-OS directory this port used before the rename took
/// over the Swift app's own `LaboLabo` directory (this module's doc
/// comment) — kept only so [`migrate_legacy_rust_data_dir`] (and
/// `TaskDatabase::default_path`'s fallback) can still find a `tasks.db`
/// written by an older build. `None` when `LABOLABO_RS_DATA_DIR` is set:
/// the override already pins a single directory ([`rust_app_data_dir_from`]
/// resolves to exactly that path too), so there is no separate "old"
/// location to migrate from or fall back to.
pub(super) fn legacy_rust_app_data_dir_from(override_dir: Option<&OsStr>) -> Option<PathBuf> {
    if override_dir.is_some_and(|dir| !dir.is_empty()) {
        return None;
    }
    Some(platform_leaf_dir("LaboLabo-rs"))
}

/// One-time startup migration (call once, early — before any
/// `TaskDatabase::default_path`/`open` call, see `main.rs`): if this port's
/// pre-1.1.0 `tasks.db` exists at the legacy directory
/// (`legacy_rust_app_data_dir_from`, `.../LaboLabo-rs/tasks.db`) and no
/// `tasks.db` exists yet at the new, Swift-shared directory
/// ([`rust_app_data_dir`], `.../LaboLabo/tasks.db`), **moves** (renames,
/// never copies) the file across.
///
/// A no-op in every other case — covers both "nothing to migrate" (fresh
/// install, or a machine that never ran a pre-1.1.0 build) and "already
/// migrated" (a previous launch already moved it, or something already
/// created a file at the new path) without any separate persisted "have I
/// migrated" flag: once a rename succeeds the legacy file is gone, so every
/// later call — including the very next launch — just sees "nothing to
/// migrate" again.
///
/// Never touched when `LABOLABO_RS_DATA_DIR` is set
/// (`legacy_rust_app_data_dir_from`'s doc comment).
///
/// On failure (parent directory creation, or the rename itself — e.g. no
/// write permission, an exotic cross-device setup) this does **not** retry
/// or surface an error: it just leaves the legacy file where it is, to be
/// picked up by a later launch once whatever blocked the rename clears (or
/// left there forever otherwise). `TaskDatabase::default_path()` falls back
/// to the legacy path whenever the new one doesn't exist yet (see its doc
/// comment), so the app keeps working against the pre-existing database
/// either way — a failed migration degrades to "the rename didn't happen
/// yet", never to data loss or a fresh empty database shadowing real data.
pub fn migrate_legacy_rust_data_dir() {
    let override_dir = std::env::var_os("LABOLABO_RS_DATA_DIR");
    let override_dir = override_dir.as_deref();
    if let Some(legacy_dir) = legacy_rust_app_data_dir_from(override_dir) {
        let new_dir = rust_app_data_dir_from(override_dir);
        migrate_task_db(
            &legacy_dir.join(TASK_DB_FILE_NAME),
            &new_dir.join(TASK_DB_FILE_NAME),
        );
    }
}

/// The path-pair core of [`migrate_legacy_rust_data_dir`], split out so
/// tests can point it at tempdir fixtures instead of the real per-user data
/// directory (same "_from"-style convention as this module's other testable
/// cores).
fn migrate_task_db(legacy_path: &Path, new_path: &Path) {
    if new_path.exists() || !legacy_path.exists() {
        return;
    }
    if let Some(parent) = new_path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let _ = std::fs::rename(legacy_path, new_path);
}

#[cfg(unix)]
fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

// `platform_leaf_dir`'s `#[cfg(target_os = "windows")]` branch above
// already falls back to this when `APPDATA` is unset -- that fallback
// exists but had no Windows-side `home_dir()` to call (a pre-existing gap:
// nothing had ever compiled this module for `target_os = "windows"` before
// this CI wave), which failed to compile. `USERPROFILE`
// is Windows' closest analog to Unix's `$HOME` (the user's profile
// directory, e.g. `C:\Users\<name>`); `C:\` is the same
// "give up, use the drive root" last resort `home_dir`'s Unix arm uses
// (`/`).
#[cfg(windows)]
fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_with_labolabo() {
        assert_eq!(app_data_dir().file_name().unwrap(), "LaboLabo");
    }

    #[test]
    fn rust_app_data_dir_now_matches_swift_app_data_dir() {
        // As of the 1.1.0 rename the two functions resolve to the *same*
        // directory -- this module's doc comment covers why that's safe
        // (different database filenames, not different directories, keep
        // the two apps' files from colliding).
        assert_eq!(rust_app_data_dir(), app_data_dir());
        assert_eq!(rust_app_data_dir().file_name().unwrap(), "LaboLabo");
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
            "LaboLabo"
        );
    }

    // --- legacy_rust_app_data_dir_from -------------------------------------

    #[test]
    fn legacy_dir_is_the_pre_1_1_0_leaf() {
        assert_eq!(
            legacy_rust_app_data_dir_from(None)
                .unwrap()
                .file_name()
                .unwrap(),
            "LaboLabo-rs"
        );
    }

    #[test]
    fn legacy_dir_is_none_when_the_override_is_set() {
        assert!(
            legacy_rust_app_data_dir_from(Some(std::ffi::OsStr::new("/tmp/labolabo-scratch")))
                .is_none()
        );
    }

    #[test]
    fn legacy_dir_ignores_an_empty_override() {
        assert!(legacy_rust_app_data_dir_from(Some(std::ffi::OsStr::new(""))).is_some());
    }

    // --- migrate_task_db -- tempdir combinations ----------------------------
    //
    // `migrate_task_db` is the path-pair core of `migrate_legacy_rust_data_dir`
    // (env/override handling lives one layer up, already covered by the
    // `legacy_dir_*` tests above); these exercise every legacy/new
    // existence combination directly against tempdir fixtures, never the
    // real per-user data directory.

    fn scratch_dir(prefix: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn migrate_is_a_no_op_when_neither_file_exists() {
        let dir = scratch_dir("labolabo-core-data-dir-migrate-neither");
        let legacy = dir.join("legacy").join(TASK_DB_FILE_NAME);
        let new = dir.join("new").join(TASK_DB_FILE_NAME);

        migrate_task_db(&legacy, &new);

        assert!(!legacy.exists());
        assert!(!new.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_moves_the_legacy_file_when_only_it_exists() {
        let dir = scratch_dir("labolabo-core-data-dir-migrate-legacy-only");
        let legacy_dir = dir.join("legacy");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let legacy = legacy_dir.join(TASK_DB_FILE_NAME);
        std::fs::write(&legacy, b"legacy-db-bytes").unwrap();
        let new = dir.join("new").join(TASK_DB_FILE_NAME);

        migrate_task_db(&legacy, &new);

        assert!(!legacy.exists(), "legacy file should be moved, not copied");
        assert!(new.exists());
        assert_eq!(std::fs::read(&new).unwrap(), b"legacy-db-bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_leaves_the_new_file_alone_when_only_it_exists() {
        let dir = scratch_dir("labolabo-core-data-dir-migrate-new-only");
        let new_dir = dir.join("new");
        std::fs::create_dir_all(&new_dir).unwrap();
        let new = new_dir.join(TASK_DB_FILE_NAME);
        std::fs::write(&new, b"new-db-bytes").unwrap();
        let legacy = dir.join("legacy").join(TASK_DB_FILE_NAME);

        migrate_task_db(&legacy, &new);

        assert!(!legacy.exists());
        assert_eq!(std::fs::read(&new).unwrap(), b"new-db-bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_prefers_the_new_file_and_leaves_the_legacy_one_orphaned_when_both_exist() {
        let dir = scratch_dir("labolabo-core-data-dir-migrate-both");
        let legacy_dir = dir.join("legacy");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let legacy = legacy_dir.join(TASK_DB_FILE_NAME);
        std::fs::write(&legacy, b"legacy-db-bytes").unwrap();
        let new_dir = dir.join("new");
        std::fs::create_dir_all(&new_dir).unwrap();
        let new = new_dir.join(TASK_DB_FILE_NAME);
        std::fs::write(&new, b"new-db-bytes").unwrap();

        migrate_task_db(&legacy, &new);

        // Neither file is touched -- the new path already had a database,
        // so it wins outright; the legacy file is left in place rather than
        // silently deleted or overwritten.
        assert!(legacy.exists());
        assert_eq!(std::fs::read(&legacy).unwrap(), b"legacy-db-bytes");
        assert_eq!(std::fs::read(&new).unwrap(), b"new-db-bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Note: `migrate_legacy_rust_data_dir` itself (the real public entry
    // point, which reads `LABOLABO_RS_DATA_DIR` from the real process
    // environment) is intentionally *not* unit-tested directly -- this
    // crate's convention is to keep env reads in a thin, untested caller
    // and test the pure core instead (see `rust_app_data_dir_from`'s doc
    // comment). Its "no-op when the override is set" guarantee is proven
    // one layer down by `legacy_dir_is_none_when_the_override_is_set`
    // (`legacy_rust_app_data_dir_from` returns `None`) combined with
    // `migrate_legacy_rust_data_dir`'s own `if let Some(legacy_dir)` guard,
    // which unconditionally treats `None` as "nothing to migrate".
}
