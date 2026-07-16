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
//! - Windows: PATHEXT-aware `PATH` scan only (see
//!   [`locate_via_path_pathext`]) -- no fixed candidates (the unix list is
//!   Homebrew/Nix/... paths with no Windows analog) and no login-shell
//!   fallback (a Windows GUI process inherits the full user `PATH` from the
//!   registry-backed environment; there is no launchd-style stripped-PATH
//!   problem to work around).
//! - Other targets: unimplemented stub -- see `locate` below.

// ログインシェル解決（locate_via_login_shell）が macOS 限定のため、これらの import は
// Linux ビルドでは未使用になる。clippy -D warnings（CI の ubuntu ジョブ）を通すために
// 同じ cfg でゲートする。`Path`（`PathBuf` と違いトレイトシグネチャでは使わない）も
// 同様に、実際に使う関数（is_executable_file/locate_fixed_or_path/
// locate_via_login_shell）がすべて unix 限定のため `#[cfg(unix)]` でゲートする
// （Windows 実装 locate_via_path_pathext は `PathBuf` しか使わない）。
#[cfg(target_os = "macos")]
use crate::process;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
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
///
/// `#[cfg(unix)]`: only called from `locate_fixed_or_path`, which only the
/// macOS/Linux `ToolLocating::locate` impls call -- the `#[cfg(not(unix))]`
/// impl below is a Windows stub that never reaches it. Ungated, this (and
/// its callees) would be `dead_code` on Windows.
#[cfg(unix)]
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
///
/// `#[cfg(unix)]`: same "only reachable from the macOS/Linux `locate`
/// impls" reasoning as `fixed_candidates` above -- its own body already had
/// a `#[cfg(not(unix))]` arm (a `true` stub) that was *never actually
/// reachable*, since nothing on a non-unix target called this function in
/// the first place; that dead branch is removed here rather than kept
/// around unreachable.
#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}

/// Fixed candidates, then a linear scan of `PATH`. Shared by every
/// platform's `locate` (macOS layers the login-shell fallback on top).
#[cfg(unix)]
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

/// The `PATHEXT` extension list (`.COM;.EXE;.BAT;.CMD;...`), lowercased and
/// including the leading dot, falling back to the OS default four when the
/// variable is unset/empty -- the same fallback `cmd.exe` itself applies.
#[cfg(windows)]
fn pathext_extensions() -> Vec<String> {
    let raw = std::env::var("PATHEXT").unwrap_or_default();
    let exts: Vec<String> = raw
        .split(';')
        .filter(|e| !e.is_empty())
        .map(|e| e.to_lowercase())
        .collect();
    if exts.is_empty() {
        return [".com", ".exe", ".bat", ".cmd"]
            .into_iter()
            .map(str::to_string)
            .collect();
    }
    exts
}

/// Windows resolution: a linear scan of `PATH` (`std::env::split_paths`,
/// i.e. `;`-separated), trying `<dir>\<name><ext>` for every `PATHEXT`
/// extension -- plus `<dir>\<name>` as-is first when `name` already carries
/// one of those extensions (`locate("cmd.exe")` must not require a
/// `cmd.exe.exe`). This mirrors what `where <name>`/`CreateProcessW`'s
/// search does, without shelling out to `where`: the search rule is simple
/// enough that a subprocess (plus its own PATH/quoting concerns) buys
/// nothing -- see the porting brief ("PATH 直接スキャン + PATHEXT が素直なら
/// where 不要 -- 実装の単純さ優先").
///
/// "Executable" on Windows means "is a regular file with an executable
/// extension" -- there is no mode-bit check to port `is_executable_file`'s
/// `0o111` test to, and `PATHEXT` membership is exactly the executability
/// rule the shell itself uses.
#[cfg(windows)]
fn locate_via_path_pathext(name: &str) -> Option<PathBuf> {
    let exts = pathext_extensions();
    let name_lower = name.to_lowercase();
    let has_executable_extension = exts.iter().any(|ext| name_lower.ends_with(ext.as_str()));

    let path_env = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_env) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        if has_executable_extension {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        for ext in &exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
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

    #[cfg(windows)]
    fn locate(&self, name: &str) -> Option<PathBuf> {
        // Windows: PATHEXT-aware PATH scan only -- see the module doc
        // comment for why there are no fixed candidates and no login-shell
        // fallback here.
        locate_via_path_pathext(name)
    }

    #[cfg(not(any(unix, windows)))]
    fn locate(&self, _name: &str) -> Option<PathBuf> {
        // No other target currently builds this crate; kept as a loud stub
        // rather than a silent None so a new platform port starts here
        // deliberately.
        unimplemented!("ToolLocator::locate is not implemented for this platform")
    }
}

// Unix-only because the tests probe unix base binaries (`sh`/`ls`) and the
// unix-specific `is_executable_file` -- the Windows implementation has its
// own test module below.
#[cfg(all(test, unix))]
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

// Windows counterparts of the unix tests above, probing binaries that exist
// on every Windows installation (`cmd`) -- these run for real on the `rust
// (windows-latest)` CI job.
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn locate_cmd_without_extension_resolves_via_pathext() {
        let path = ToolLocator.locate("cmd");
        let path = path.expect("base binary `cmd` should resolve");
        assert!(path.is_absolute());
        assert!(path.is_file());
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
        assert_eq!(ext.as_deref(), Some("exe"), "cmd should resolve to cmd.exe");
    }

    #[test]
    fn locate_cmd_with_explicit_extension_resolves_as_is() {
        let path = ToolLocator.locate("cmd.exe");
        let path = path.expect("`cmd.exe` should resolve");
        assert!(path.is_file());
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .eq_ignore_ascii_case("cmd.exe"));
    }

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
    fn locate_name_with_path_separators_does_not_crash() {
        assert!(ToolLocator.locate("..\\windows\\no-such-tool").is_none());
        assert!(ToolLocator.locate("no.such.tool/here").is_none());
    }
}
