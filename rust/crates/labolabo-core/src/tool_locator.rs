//! Faithful port of
//! `Sources/LaboLaboEngine/Process/{ToolLocating,ToolLocator}.swift`.
//!
//! Resolves an external CLI tool name (`git`/`gh`/`claude`/...) to an
//! absolute path. GUI apps inherit a minimal `launchd`-provided `PATH`
//! (`/usr/bin:/bin:...`), so this tries, in order: (1) well-known fixed
//! install locations (Homebrew/Nix/mise/...), (2) the process's own `PATH`,
//! and, macOS only, (3) the user's login shell's `PATH`
//! (`$SHELL -l -c 'command -v <name>'`) -- closest to what a real terminal
//! session would resolve, so a "doctor" check and the engine's actual
//! ability to run a tool stay consistent.
//!
//! `#[cfg(target_os = ...)]`-gated per the porting brief:
//! - macOS: fixed candidates -> PATH -> login shell (full Swift behavior).
//! - Linux: fixed candidates -> PATH only (no login-shell fallback).
//! - Other targets (Windows, ...): unimplemented stub -- see `locate` below.

// ログインシェル解決（locate_via_login_shell）が macOS 限定のため、これらの import は
// Linux ビルドでは未使用になる。clippy -D warnings（CI の ubuntu ジョブ）を通すために
// 同じ cfg でゲートする。
#[cfg(target_os = "macos")]
use crate::process;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::time::Duration;

/// Abstraction over tool-name -> absolute-path resolution, so tests (and a
/// future non-`ToolLocator` implementation, e.g. for Windows) can inject a
/// fake. Mirrors the Swift `ToolLocating` protocol used as a static type
/// parameter (`locator: ToolLocating.Type = ToolLocator.self`); Rust has no
/// direct equivalent to that static-dispatch-with-a-default-type pattern for
/// free functions, so this crate uses a plain object-safe trait invoked via
/// `&dyn ToolLocating` instead (see `git_runner::run_with_locator`).
pub trait ToolLocating {
    /// Returns `name`'s absolute path, or `None` if it couldn't be found.
    fn locate(&self, name: &str) -> Option<PathBuf>;
}

/// The real resolver: fixed candidates -> PATH -> (macOS only) login shell.
pub struct ToolLocator;

/// Fixed candidate directories (Homebrew / Nix / mise shims / `~/.local` /
/// ...), unioned. Kept byte-for-byte identical to the Swift source's list
/// ("そのまま" per the porting brief) on every platform this module
/// supports -- harmless on Linux even where a given path (e.g.
/// `/opt/homebrew`) never exists, since `is_executable_file` just returns
/// `false` for it.
fn fixed_candidates(name: &str) -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    [
        format!("/opt/homebrew/bin/{name}"),
        format!("/usr/local/bin/{name}"),
        format!("/usr/bin/{name}"),
        format!("/run/current-system/sw/bin/{name}"), // Nix
        format!("{home}/.local/bin/{name}"),
        format!("{home}/.claude/local/{name}"),
        format!("{home}/.local/share/mise/shims/{name}"),
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

/// Mirrors `FileManager.default.isExecutableFile(atPath:)`: the path must
/// exist, be a regular file, and have at least one executable bit set. This
/// is a simplification of Foundation's check (which ultimately calls
/// POSIX `access(path, X_OK)`, so it also accounts for the *calling
/// process's* uid/gid against the file's owner/group -- this instead just
/// inspects the raw mode bits) but is equivalent for every path this
/// resolver actually probes (world- or group-executable binaries installed
/// by a package manager).
fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Fixed candidates, then a linear scan of `PATH`. Shared by every
/// platform's `locate` (macOS layers the login-shell fallback on top).
fn locate_fixed_or_path(name: &str) -> Option<PathBuf> {
    for candidate in fixed_candidates(name) {
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    if let Ok(path_env) = std::env::var("PATH") {
        for dir in path_env.split(':') {
            let candidate = Path::new(dir).join(name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// `$SHELL -l -c 'command -v <name>'`. Login-profile output (`.zprofile`
/// etc.) can precede the real answer, so the **last** absolute (`/`-leading)
/// line is taken. A 5s timeout guards against a hanging login shell.
#[cfg(target_os = "macos")]
fn locate_via_login_shell(name: &str) -> Option<PathBuf> {
    // `name` is always a caller-supplied constant (git/gh/claude) in
    // practice, but this allow-list is kept as defense in depth against
    // smuggling a `-c` payload through it, mirroring the Swift source.
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let out = process::run_with_timeout(
        Path::new(&shell),
        &[
            "-l".to_string(),
            "-c".to_string(),
            format!("command -v {name} 2>/dev/null"),
        ],
        None,
        None,
        Duration::from_secs(5),
    )
    .ok()
    .flatten()?;
    if out.status != 0 {
        return None;
    }
    out.stdout
        .lines()
        .map(str::trim)
        .rfind(|line| line.starts_with('/'))
        .map(PathBuf::from)
}

impl ToolLocating for ToolLocator {
    #[cfg(target_os = "macos")]
    fn locate(&self, name: &str) -> Option<PathBuf> {
        locate_fixed_or_path(name).or_else(|| locate_via_login_shell(name))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn locate(&self, name: &str) -> Option<PathBuf> {
        // Linux: fixed candidates + PATH only, no login-shell fallback --
        // see the porting brief and the module doc comment.
        locate_fixed_or_path(name)
    }

    #[cfg(not(unix))]
    fn locate(&self, _name: &str) -> Option<PathBuf> {
        // Windows isn't implemented yet. A faithful port would shell out to
        // `where <name>` (the closest analog to `command -v`) and be
        // PATHEXT-aware (.exe/.cmd/.bat/...) when probing the fixed
        // candidates, since Windows has no single "executable bit" the way
        // `is_executable_file` checks here. Deferred until a Windows target
        // actually needs this crate -- see rust/README.md's known-scope-limits
        // section.
        unimplemented!("ToolLocator::locate is not implemented for Windows yet")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from Tests/LaboLaboEngineTests/ToolLocatorTests.swift.

    #[test]
    fn locate_base_binary_returns_executable_path() {
        let path = ToolLocator.locate("sh");
        let path = path.expect("base binary `sh` should resolve");
        assert!(path.is_absolute());
        assert!(is_executable_file(&path));
    }

    #[test]
    fn locate_another_base_binary_points_to_existing_executable() {
        let path = ToolLocator.locate("ls");
        let path = path.expect("base binary `ls` should resolve");
        assert!(path.is_file());
        assert!(is_executable_file(&path));
    }

    // No CI skip here (unlike the Swift test): `run_with_timeout` bounds the
    // login shell invocation to 5s regardless of environment, so there is no
    // hang risk to guard against.
    #[test]
    fn locate_absent_tool_returns_none() {
        let name = format!(
            "labolabo-no-such-tool-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        assert!(ToolLocator.locate(&name).is_none());
    }

    #[test]
    fn locate_name_failing_login_shell_allow_list_does_not_crash() {
        // Contains a space, so the login-shell allow-list rejects it; falls
        // back to fixed-candidate/PATH resolution, which also won't match.
        let result = ToolLocator.locate("bad name");
        if let Some(path) = result {
            assert!(is_executable_file(&path));
        }
    }

    #[test]
    fn locate_name_with_path_separators_does_not_crash() {
        assert!(ToolLocator.locate("../etc/passwd").is_none());
        assert!(ToolLocator.locate("no.such.tool/here").is_none());
    }
}
