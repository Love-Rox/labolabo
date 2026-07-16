//! Injects two compile-time constants into the binary:
//!
//! - `LABOLABO_BUILD_NUMBER` -- the same `git rev-list --count HEAD`
//!   convention the Swift app (`app/project.yml`'s archive-time
//!   `CFBundleVersion` injection) and the Rust packaging scripts
//!   (`rust/scripts/bundle-macos.sh`'s `BUILD_NUMBER`, etc.) both use, so
//!   the About panel (`crate::menus::render_about_overlay`) shows the same
//!   monotonic counter a packaged build carries.
//! - `LABOLABO_RS_VERSION` -- the marketing version (`crate::menus::
//!   APP_VERSION`, RC release wave). Single-sourced from `rust/VERSION`
//!   (one plain-text line, e.g. `1.0.0-rc.1`) so the compiled binary's
//!   About panel and the packaging scripts (`bundle-macos.sh`/
//!   `package-linux.sh`/`package-windows.ps1`) never drift apart. A
//!   `LABOLABO_RS_VERSION` env var, if set, wins over the file -- this is
//!   how `.github/workflows/rust-release.yml` stamps a version derived
//!   from its `tag` input (e.g. tag `rs-v1.0.0-rc.2` -> version
//!   `1.0.0-rc.2`) into both the packaged artifact's file name *and* the
//!   binary compiled into it, without editing the checked-in `VERSION`
//!   file per release. See `rust/README.md`'s "RC リリース手順" section.
//!
//! Both are best-effort by design: building outside a git checkout / with
//! no `VERSION` file present falls back to a literal default rather than
//! failing the build -- this is display-only metadata, not something that
//! should block `cargo build`.

use std::path::Path;

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

    // Version resolution: env override (set by the packaging scripts, in
    // turn set by rust-release.yml from its `tag` input) > `rust/VERSION`
    // file > a hardcoded last-resort default (only reachable if someone
    // deletes the VERSION file -- a meaningful fallback so `cargo build`
    // still succeeds rather than failing outright).
    println!("cargo:rerun-if-env-changed=LABOLABO_RS_VERSION");
    println!("cargo:rerun-if-changed=../../VERSION");
    let version = std::env::var("LABOLABO_RS_VERSION")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::fs::read_to_string(Path::new("../../VERSION"))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "0.0.0-unknown".to_string());
    println!("cargo:rustc-env=LABOLABO_RS_VERSION={version}");
}
