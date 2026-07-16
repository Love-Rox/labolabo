//! Injects the build number into the binary as `LABOLABO_BUILD_NUMBER` --
//! the same `git rev-list --count HEAD` convention the Swift app
//! (`app/project.yml`'s archive-time `CFBundleVersion` injection) and the
//! Rust bundle script (`rust/scripts/bundle-macos.sh`'s `BUILD_NUMBER`) both
//! use, so the About panel (`crate::menus::render_about_overlay`) shows the
//! same monotonic counter a bundled build's Info.plist carries.
//!
//! Best-effort by design: building outside a git checkout (e.g. a source
//! tarball) or without `git` on PATH falls back to `"0"` rather than
//! failing the build -- the build number is display-only metadata.

fn main() {
    let build_number = std::process::Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "0".to_string());
    println!("cargo:rustc-env=LABOLABO_BUILD_NUMBER={build_number}");

    // Re-run when HEAD moves so an incremental rebuild picks up the new
    // count. In a plain checkout `.git` is a directory (watch its HEAD
    // file); in a `git worktree` checkout `.git` is a file -- watch that
    // instead. Missing paths are harmless (cargo just ignores them).
    println!("cargo:rerun-if-changed=../../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../../.git");
}
