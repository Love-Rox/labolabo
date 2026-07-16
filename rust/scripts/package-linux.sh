#!/usr/bin/env bash
# Builds the Rust port (labolabo-app / labolabo / labolabo-hook) and packages
# them into a portable Linux tarball -- the Linux counterpart of
# `bundle-macos.sh`'s macOS `.app` bundle (see rust/README.md's "Wave 6a"
# section for that one). There is no macOS-style bundle format on Linux, so
# this produces a flat `bin/` + a freedesktop.org `.desktop` launcher + an
# `install.sh` that wires the two together into the user's own
# `~/.local/share`/`~/.local/bin` (the portable-install convention most
# distros' desktop environments already scan without root) -- see "Linux"
# in `crates/labolabo-app/README.md` for the full rationale and this wave's
# known limitations (GUI launch is unverified -- built and headless-tested
# in CI only, see that section).
#
# Usage: rust/scripts/package-linux.sh [version]
#   version: optional -- see bundle-macos.sh's usage comment for the exact
#            resolution order and how it also stamps the compiled binary's
#            own About-panel version (LABOLABO_RS_VERSION).
# Output: rust/target/package/LaboLabo-linux-<version>-<arch>.tar.gz
#
# 1.1.0 rename ("LaboLabo-rs" -> "LaboLabo", Swift 版引退に伴う正式名化):
# the *user-visible* names change -- the tarball/staging name and the
# .desktop launcher's `Name=` -- but the *on-disk installation* names
# (labolabo-rs.desktop / labolabo-rs.png / ~/.local/share/labolabo-rs)
# deliberately do NOT: keeping them means a user upgrading over a pre-rename
# install overwrites the same .desktop/icon/install-dir in place (one
# launcher entry, now titled "LaboLabo") instead of accumulating a stale
# "LaboLabo-rs" duplicate next to a new "LaboLabo" one. Same reasoning as
# keeping the three executable names (bundle-macos.sh's rename comment).
set -euo pipefail

if [ "$(uname -s)" != "Linux" ]; then
    echo "error: package-linux.sh must run on Linux (got $(uname -s))" >&2
    exit 1
fi

# Resolve paths relative to this script, not the caller's cwd -- same
# rationale as bundle-macos.sh (works from either `rust/` or the repo root).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$RUST_DIR/.." && pwd)"

# --- Version -------------------------------------------------------------
#
# Same marketing version as the macOS bundle (`bundle-macos.sh`) -- one
# version number across every platform's packaged artifact, deliberately
# decoupled from the workspace crates' own (unbumped, pre-1.0) Cargo.toml
# `version`. Same resolution order as `bundle-macos.sh` ($1 > env >
# rust/VERSION file > literal fallback) -- see that script's comment for
# the full rationale, including why this is exported before `cargo build`.
VERSION="${1:-${LABOLABO_RS_VERSION:-$(cat "$RUST_DIR/VERSION" 2>/dev/null | tr -d '[:space:]')}}"
if [ -z "$VERSION" ]; then
    VERSION="1.0.0-rc.1"
fi
export LABOLABO_RS_VERSION="$VERSION"
ARCH="$(uname -m)"

PACKAGE_DIR="$RUST_DIR/target/package"
STAGE_NAME="LaboLabo-linux-$VERSION-$ARCH"
STAGE_DIR="$PACKAGE_DIR/$STAGE_NAME"

# --- VT backend selection --------------------------------------------------
#
# Same default-to-ghostty-vt policy as `bundle-macos.sh` -- see that
# script's "VT backend selection" section for the full rationale (Ghostty
# identity as the production VT core vs. the Zig-free `backend-alacritty`
# dev default) and `rust/README.md`'s "配布 vs 開発の既定バックエンド"
# section. `LABOLABO_VT_BACKEND=alacritty` is the same escape hatch.
VT_BACKEND="${LABOLABO_VT_BACKEND:-ghostty}"
CARGO_FEATURE_ARGS=""
case "$VT_BACKEND" in
    ghostty)
        if ! command -v zig >/dev/null 2>&1; then
            cat >&2 <<EOF
error: the ghostty-vt backend requires Zig 0.16 on PATH, but no 'zig' binary was found.

Set up:
  1. Install Zig 0.16.0 (https://ziglang.org/download/) and put it on PATH
     ('zig version' must print 0.16.x).
  2. Clone the Zig-0.16-compatible Ghostty fork this project pins in CI (see
     .github/workflows/ci.yml's 'rust-term-ghostty' job -- vancluever/ghostty,
     GHOSTTY_REF for the exact pinned commit) and export:
       export GHOSTTY_SOURCE_DIR=/path/to/that/checkout

Or fall back to the alacritty backend (not the intended production backend --
see rust/README.md):
  LABOLABO_VT_BACKEND=alacritty $0 ${1:-}
EOF
            exit 1
        fi
        ZIG_VERSION="$(zig version)"
        case "$ZIG_VERSION" in
            0.16.*) ;;
            *)
                echo "error: the ghostty-vt backend requires Zig 0.16.x; found '$(command -v zig)' reporting version $ZIG_VERSION. Put a 0.16.x zig first on PATH, or set LABOLABO_VT_BACKEND=alacritty to fall back." >&2
                exit 1
                ;;
        esac
        if [ -z "${GHOSTTY_SOURCE_DIR:-}" ] || [ ! -f "${GHOSTTY_SOURCE_DIR}/build.zig" ]; then
            cat >&2 <<EOF
error: the ghostty-vt backend requires GHOSTTY_SOURCE_DIR to point at a
Ghostty source checkout containing build.zig, but it is $( [ -z "${GHOSTTY_SOURCE_DIR:-}" ] && echo "unset" || echo "set to '$GHOSTTY_SOURCE_DIR', which has no build.zig" ).

Clone the Zig-0.16-compatible fork this project pins in CI (see
.github/workflows/ci.yml's 'rust-term-ghostty' job -- vancluever/ghostty,
GHOSTTY_REF for the exact pinned commit) and export:
  export GHOSTTY_SOURCE_DIR=/path/to/that/checkout

Or fall back to the alacritty backend (not the intended production backend --
see rust/README.md):
  LABOLABO_VT_BACKEND=alacritty $0 ${1:-}
EOF
            exit 1
        fi
        CARGO_FEATURE_ARGS="--no-default-features --features backend-ghostty-vt"
        VT_BACKEND_LABEL="libghostty-vt"
        ;;
    alacritty)
        VT_BACKEND_LABEL="alacritty (fallback -- see rust/README.md)"
        ;;
    *)
        echo "error: unknown LABOLABO_VT_BACKEND '$VT_BACKEND' (expected 'ghostty' or 'alacritty')" >&2
        exit 1
        ;;
esac
echo "==> VT backend: $VT_BACKEND_LABEL"

echo "==> cargo build --release (labolabo-app, labolabo, labolabo-hook), version $VERSION"
# Same two `-p` flags as bundle-macos.sh: `-p labolabo-app` builds this
# package's two bin targets (labolabo-app, the gpui GUI; labolabo, the
# control CLI); `-p labolabo-core` additionally builds its own
# `src/bin/labolabo-hook.rs` (the hooks forwarder). `$CARGO_FEATURE_ARGS`
# selects the VT backend -- see "VT backend selection" above.
# shellcheck disable=SC2086
(cd "$RUST_DIR" && cargo build --release -p labolabo-app -p labolabo-core $CARGO_FEATURE_ARGS)

BUILD_DIR="$RUST_DIR/target/release"
for bin in labolabo-app labolabo labolabo-hook; do
    if [ ! -x "$BUILD_DIR/$bin" ]; then
        echo "error: expected binary not found after build: $BUILD_DIR/$bin" >&2
        exit 1
    fi
done

echo "==> Assembling $STAGE_DIR (version $VERSION, arch $ARCH)"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/bin"

# All three binaries live side by side in bin/, same layout convention as
# the macOS bundle's flat Contents/MacOS/ -- labolabo-app's hooks
# integration (`crates/labolabo-app/src/hooks.rs`'s `resolve_hook_binary`)
# finds `labolabo-hook` as the sibling of `std::env::current_exe()`, so
# this is load-bearing, not just tidiness.
cp "$BUILD_DIR/labolabo-app" "$STAGE_DIR/bin/labolabo-app"
cp "$BUILD_DIR/labolabo" "$STAGE_DIR/bin/labolabo"
cp "$BUILD_DIR/labolabo-hook" "$STAGE_DIR/bin/labolabo-hook"
chmod +x "$STAGE_DIR/bin/labolabo-app" "$STAGE_DIR/bin/labolabo" "$STAGE_DIR/bin/labolabo-hook"

# --- Icon ------------------------------------------------------------------
#
# One plain PNG (freedesktop.org icon themes accept PNG directly -- no
# `.icns`-style conversion step needed, unlike bundle-macos.sh). Reuses the
# Swift app's own 512x512 artwork (same "must not ship unbranded/placeholder
# icons" direction bundle-macos.sh's icon section documents) rather than
# drawing something new for the Rust port.
ICON_SRC="$REPO_ROOT/app/Sources/Assets.xcassets/AppIcon.appiconset/icon_512x512.png"
if [ ! -f "$ICON_SRC" ]; then
    echo "error: Swift app icon source not found: $ICON_SRC" >&2
    exit 1
fi
cp "$ICON_SRC" "$STAGE_DIR/labolabo-rs.png"

# --- .desktop launcher + installer -----------------------------------------
#
# A `.desktop` file's `Exec=` needs an absolute path -- there is no portable
# "relative to this file" syntax freedesktop.org desktop environments honor
# -- so a `.desktop` shipped inside a tarball that can be extracted anywhere
# can't hardcode one. `install.sh` fills in the real path at install time
# (via `sed`) once the user has decided where the tarball lives, then drops
# the finished file into `~/.local/share/applications` (and symlinks the
# binaries into `~/.local/bin`) -- the standard root-less, distro-agnostic
# "install for this user only" convention every major desktop environment's
# app launcher already scans, no `sudo`/package manager needed.
cat > "$STAGE_DIR/labolabo-rs.desktop.in" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=LaboLabo
Comment=Terminal + live Git status side by side, for running AI coding agents in parallel worktrees
Exec=@EXEC@
Icon=@ICON@
Terminal=false
Categories=Development;Utility;
DESKTOP

cat > "$STAGE_DIR/install.sh" <<'INSTALL'
#!/usr/bin/env bash
# Installs LaboLabo for the current user only (no root needed): copies
# bin/ into ~/.local/share/labolabo-rs, symlinks labolabo-app/labolabo into
# ~/.local/bin (drop that on your PATH if it isn't already), installs the
# icon under the freedesktop.org hicolor icon theme, and writes a finished
# .desktop launcher into ~/.local/share/applications. (The on-disk
# labolabo-rs paths are kept from the pre-1.1.0 "LaboLabo-rs" releases on
# purpose, so upgrading over an old install replaces it in place -- see the
# packaging script's rename comment in the source repo.)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="${LABOLABO_RS_INSTALL_DIR:-$HOME/.local/share/labolabo-rs}"
BIN_DIR="$HOME/.local/bin"
ICON_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"
APPS_DIR="$HOME/.local/share/applications"

mkdir -p "$INSTALL_DIR" "$BIN_DIR" "$ICON_DIR" "$APPS_DIR"
cp -f "$SCRIPT_DIR/bin/labolabo-app" "$SCRIPT_DIR/bin/labolabo" "$SCRIPT_DIR/bin/labolabo-hook" "$INSTALL_DIR/"
chmod +x "$INSTALL_DIR/labolabo-app" "$INSTALL_DIR/labolabo" "$INSTALL_DIR/labolabo-hook"
cp -f "$SCRIPT_DIR/labolabo-rs.png" "$ICON_DIR/labolabo-rs.png"
ln -sf "$INSTALL_DIR/labolabo-app" "$BIN_DIR/labolabo-app"
ln -sf "$INSTALL_DIR/labolabo" "$BIN_DIR/labolabo"
sed -e "s|@EXEC@|$INSTALL_DIR/labolabo-app|" -e "s|@ICON@|labolabo-rs|" \
    "$SCRIPT_DIR/labolabo-rs.desktop.in" > "$APPS_DIR/labolabo-rs.desktop"
chmod +x "$APPS_DIR/labolabo-rs.desktop"

command -v update-desktop-database >/dev/null 2>&1 &&
    update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true

echo "Installed to $INSTALL_DIR"
echo "  - Run directly:        $BIN_DIR/labolabo-app"
echo "  - Or from the app menu: LaboLabo (log out/in first if it doesn't show up yet)"
echo "Make sure $BIN_DIR is on your PATH to use 'labolabo-app'/'labolabo' by name."
INSTALL
chmod +x "$STAGE_DIR/install.sh"

# --- README (tarball-local; see crates/labolabo-app/README.md's "Linux"
# section for the full picture -- build deps, known limitations) ----------
cat > "$STAGE_DIR/README.md" <<README
# LaboLabo $VERSION ($ARCH) -- Linux package

## Install (no root needed)

\`\`\`sh
./install.sh
\`\`\`

Copies \`bin/\` to \`~/.local/share/labolabo-rs\`, symlinks \`labolabo-app\`/
\`labolabo\` into \`~/.local/bin\`, and installs an application-menu launcher
(icon + \`.desktop\` entry) for the current user only.

## Run without installing

\`\`\`sh
./bin/labolabo-app
\`\`\`

## What's inside

- \`bin/labolabo-app\` -- the gpui terminal-shell GUI.
- \`bin/labolabo\` -- the control CLI (\`labolabo tab open\`, etc. -- see
  \`docs/control-protocol.md\` in the source repo).
- \`bin/labolabo-hook\` -- the Claude Code hooks forwarder; must stay next to
  \`labolabo-app\` (hooks integration finds it as its sibling binary).
- \`labolabo-rs.png\`, \`labolabo-rs.desktop.in\` -- launcher icon/template,
  filled in and installed by \`install.sh\`.

## Known limitations

This build is produced and headless-tested (\`cargo build\`/\`clippy\`/\`cargo
test\`, no window ever opened) by CI on \`ubuntu-latest\`; **actual GUI
display on a real X11/Wayland desktop has not been verified** -- there is no
Linux machine in this project's own development loop yet. See
\`crates/labolabo-app/README.md\`'s "Linux" section (in the source repo) for
the full list of known gaps ("IDE で開く" is unavailable, etc.) and required
system libraries if you're building from source instead of using this
prebuilt tarball.
README

# --- tar.gz ------------------------------------------------------------
TARBALL="$PACKAGE_DIR/$STAGE_NAME.tar.gz"
rm -f "$TARBALL"
(cd "$PACKAGE_DIR" && tar -czf "$STAGE_NAME.tar.gz" "$STAGE_NAME")

echo "==> Done"
echo "    Staged: $STAGE_DIR"
echo "    Tarball: $TARBALL"
