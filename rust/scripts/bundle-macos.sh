#!/usr/bin/env bash
# Builds the Rust port (labolabo-app / labolabo / labolabo-hook) and packages
# them into a macOS "LaboLabo-rs.app" bundle, ad-hoc signed and zipped for
# distribution -- the Rust-side counterpart of the Swift app's
# `.github/workflows/release-build.yml` "Ad-hoc sign, zip" step (same
# `codesign -s -` + `ditto -c -k --keepParent` recipe; no Developer ID /
# notarization, by explicit decision -- see rust/README.md's bundling
# section).
#
# Usage: rust/scripts/bundle-macos.sh [version]
#   version: optional, e.g. "1.0.0-rc.2" -- overrides both this bundle's
#            CFBundleShortVersionString/zip name AND the compiled binary's
#            own About-panel version (via LABOLABO_RS_VERSION, see below).
#            Falls back to $LABOLABO_RS_VERSION if unset, then to
#            rust/VERSION's contents (this script's "current value" -- see
#            rust-release.yml's module comment for why CI always passes
#            this explicitly).
# Output: rust/target/bundle/LaboLabo-rs.app and .../LaboLabo-rs-<version>.zip
set -euo pipefail

# Resolve paths relative to this script, not the caller's cwd, so it works
# whether invoked as `./scripts/bundle-macos.sh` (cwd = rust/) or
# `rust/scripts/bundle-macos.sh` (cwd = repo root) -- both are documented
# entry points (this file and the `rust-app-bundle.yml`/`rust-release.yml`
# CI jobs).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$RUST_DIR/.." && pwd)"

BUNDLE_DIR="$RUST_DIR/target/bundle"
APP_NAME="LaboLabo-rs"
APP_BUNDLE="$BUNDLE_DIR/$APP_NAME.app"
BUNDLE_ID="com.love-rox.labolabo-rs"

# --- Version -----------------------------------------------------------
#
# CFBundleShortVersionString: **not** the workspace crates' own Cargo.toml
# `version` (still 0.1.0 -- this Rust port is pre-1.0 internally). Per
# explicit product direction, this bundle is versioned as a **major bump
# from the current Swift app's release line** (`Config/Version.xcconfig`'s
# `MARKETING_VERSION`) rather than continuing that 0.x line or reusing the
# crates' 0.1.0 -- i.e. a 1.0.0-series version, not a continuation of
# either. This is a marketing/distribution version, deliberately decoupled
# from both the Swift app's version counter and the Cargo crates' own
# (unbumped) version fields.
#
# Resolution order: CLI arg ($1) > $LABOLABO_RS_VERSION env > rust/VERSION
# file's contents > a hardcoded last-resort literal (only reached if the
# VERSION file itself is missing). `rust-release.yml` always passes $1
# explicitly (derived from its `tag` input); a plain local/manual run (or
# `rust-app-bundle.yml`'s workflow_dispatch, which predates the RC wave and
# still calls this script with no argument) falls through to the checked-in
# `rust/VERSION` file, which is this repo's single source of truth for "the
# current default version" -- see that file and `build.rs`'s doc comment.
#
# **Exported** as LABOLABO_RS_VERSION before `cargo build` below so the
# compiled binary's own About panel (`crates/labolabo-app/src/menus.rs`
# `APP_VERSION`, injected by `build.rs`) always matches this bundle's
# CFBundleShortVersionString -- no manual sync needed, unlike before this
# wave.
VERSION="${1:-${LABOLABO_RS_VERSION:-$(cat "$RUST_DIR/VERSION" 2>/dev/null | tr -d '[:space:]')}}"
if [ -z "$VERSION" ]; then
    VERSION="1.0.0-rc.1"
fi
export LABOLABO_RS_VERSION="$VERSION"

echo "==> cargo build --release (labolabo-app, labolabo, labolabo-hook), version $VERSION"
# `-p labolabo-app` builds this package's two bin targets (labolabo-app,
# the gpui GUI; labolabo, the control CLI, see its Cargo.toml). `-p
# labolabo-core` additionally builds its own `src/bin/labolabo-hook.rs` bin
# target (the hooks forwarder) -- labolabo-core is otherwise just a library
# dependency of labolabo-app, so its bin isn't produced by the first `-p`
# alone. Release, matching the Swift app's Release-configuration release
# build.
(cd "$RUST_DIR" && cargo build --release -p labolabo-app -p labolabo-core)

BUILD_DIR="$RUST_DIR/target/release"
for bin in labolabo-app labolabo labolabo-hook; do
    if [ ! -x "$BUILD_DIR/$bin" ]; then
        echo "error: expected binary not found after build: $BUILD_DIR/$bin" >&2
        exit 1
    fi
done

# CFBundleVersion (build number): same convention as the Swift app
# (`app/project.yml`'s postBuildScripts) -- the monotonic git commit count,
# not a hand-maintained counter.
BUILD_NUMBER="$(git -C "$REPO_ROOT" rev-list --count HEAD)"

echo "==> Assembling $APP_BUNDLE (version $VERSION, build $BUILD_NUMBER)"
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"

# All three binaries live side by side in Contents/MacOS/: labolabo-app's
# hooks integration (`crates/labolabo-app/src/hooks.rs`'s
# `resolve_hook_binary`) resolves `labolabo-hook` as the sibling of
# `std::env::current_exe()`, which is exactly this layout -- no code change
# needed, but this comment documents the load-bearing constraint so the
# bundle layout doesn't drift away from it.
cp "$BUILD_DIR/labolabo-app" "$APP_BUNDLE/Contents/MacOS/labolabo-app"
cp "$BUILD_DIR/labolabo" "$APP_BUNDLE/Contents/MacOS/labolabo"
cp "$BUILD_DIR/labolabo-hook" "$APP_BUNDLE/Contents/MacOS/labolabo-hook"
chmod +x "$APP_BUNDLE/Contents/MacOS/labolabo-app" \
    "$APP_BUNDLE/Contents/MacOS/labolabo" \
    "$APP_BUNDLE/Contents/MacOS/labolabo-hook"

# --- Icon ----------------------------------------------------------------
#
# Reuse the Swift app's own icon artwork (per explicit user direction: the
# Rust bundle must not ship without an icon, and must not invent new
# artwork) -- `app/Sources/Assets.xcassets/AppIcon.appiconset/*.png` already
# uses exactly `iconutil`'s expected `.iconset` naming convention
# (`icon_16x16.png`, `icon_16x16@2x.png`, ... `icon_512x512@2x.png`), so no
# resizing/renaming is needed: copy them into a scratch `.iconset` dir and
# hand that to `iconutil -c icns`.
ICON_SRC_DIR="$REPO_ROOT/app/Sources/Assets.xcassets/AppIcon.appiconset"
if [ ! -d "$ICON_SRC_DIR" ]; then
    echo "error: Swift app icon source not found: $ICON_SRC_DIR" >&2
    exit 1
fi
ICONSET_DIR="$BUNDLE_DIR/AppIcon.iconset"
rm -rf "$ICONSET_DIR"
mkdir -p "$ICONSET_DIR"
cp "$ICON_SRC_DIR"/icon_*.png "$ICONSET_DIR/"
iconutil -c icns "$ICONSET_DIR" -o "$APP_BUNDLE/Contents/Resources/AppIcon.icns"
rm -rf "$ICONSET_DIR"

# --- Info.plist ------------------------------------------------------------
cat > "$APP_BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>labolabo-app</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$BUILD_NUMBER</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <!-- gpui's build.rs sets its macOS linker version-min flag to 10.15.7
         (its own Metal-backed-renderer floor); mirror that here rather
         than the Swift app's 14.0 deployment target (app/project.yml),
         which is unrelated to this binary. -->
    <key>LSMinimumSystemVersion</key>
    <string>10.15.7</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

# --- Ad-hoc sign + zip -----------------------------------------------------
#
# Same recipe as `.github/workflows/release-build.yml`'s "Ad-hoc sign, zip,
# upload to release" step: null (`-`) signing identity, `--deep` to cover
# the bundled binaries too, `ditto` (not plain `zip`) to preserve the
# bundle's resource forks / extended attributes on extraction.
echo "==> codesign --force --deep --sign -"
codesign --force --deep --sign - "$APP_BUNDLE"
codesign --verify --strict "$APP_BUNDLE"

ZIP_PATH="$BUNDLE_DIR/$APP_NAME-$VERSION.zip"
rm -f "$ZIP_PATH"
ditto -c -k --keepParent "$APP_BUNDLE" "$ZIP_PATH"

echo "==> Done"
echo "    App: $APP_BUNDLE"
echo "    Zip: $ZIP_PATH"
