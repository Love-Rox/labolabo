# Builds the Rust port (labolabo-app / labolabo / labolabo-hook) and packages
# them into a portable Windows zip -- the Windows counterpart of
# `bundle-macos.sh`'s macOS `.app` bundle and `package-linux.sh`'s Linux
# tar.gz (see rust/README.md's "Wave 6a"/"Linux (wave 7a)" sections for
# those). There is no macOS-.app-style bundle format on Windows and no
# freedesktop.org-style `.desktop`/install.sh convention either, so this
# produces a flat `bin\` + the app icon (`.ico`, for a user to point a
# hand-made shortcut at -- see "Icon" below for why this is copy-only, not
# embedded into the .exe) + a README -- see "Windows" in
# `crates/labolabo-app/README.md` for the full rationale and this wave's
# known limitations (GUI launch is unverified -- built and headless-tested
# in CI only, see that section).
#
# Usage: pwsh rust/scripts/package-windows.ps1 [-Version <version>]
#   -Version: optional -- see bundle-macos.sh's usage comment for the exact
#             resolution order and how it also stamps the compiled binary's
#             own About-panel version (LABOLABO_RS_VERSION env var).
# Output: rust/target/package/LaboLabo-rs-windows-<version>-<arch>.zip
#
# Requires Windows (produces/copies real .exe binaries) -- run on
# `windows-latest` CI (`rust-app-bundle.yml`'s `package-windows` job, or
# `rust-release.yml`'s) or a local Windows/PowerShell dev machine, same
# constraint `bundle-macos.sh` has for macOS.
param(
    [string]$Version
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not $IsWindows) {
    Write-Error "package-windows.ps1 must run on Windows (got `$IsWindows = $IsWindows)"
    exit 1
}

# Resolve paths relative to this script, not the caller's cwd -- same
# rationale as bundle-macos.sh/package-linux.sh (works from either `rust\`
# or the repo root).
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RustDir = Resolve-Path (Join-Path $ScriptDir "..")

# --- Version ---------------------------------------------------------------
#
# Same marketing version as the macOS bundle / Linux package (`bundle-macos.sh`
# / `package-linux.sh`) -- one version number across every platform's
# packaged artifact, deliberately decoupled from the workspace crates' own
# (unbumped, pre-1.0) Cargo.toml `version`. Same resolution order as
# `bundle-macos.sh` (`-Version` param > `$env:LABOLABO_RS_VERSION` >
# `rust/VERSION` file > literal fallback) -- see that script's comment for
# the full rationale, including why this is exported (as an env var, for
# `cargo build`'s `build.rs` to pick up) before the build below.
if ($Version) {
    # explicit param wins
} elseif ($env:LABOLABO_RS_VERSION) {
    $Version = $env:LABOLABO_RS_VERSION
} else {
    $VersionFile = Join-Path $RustDir "VERSION"
    if (Test-Path $VersionFile -PathType Leaf) {
        $Version = (Get-Content $VersionFile -Raw).Trim()
    }
}
if (-not $Version) {
    $Version = "1.0.0-rc.1"
}
$env:LABOLABO_RS_VERSION = $Version
# No ARM64 Windows runner exists in this project's CI yet (`windows-latest`
# is x86_64-only) -- hardcoded rather than probed, same "one arch for now"
# simplification `package-linux.sh` avoids only because `uname -m` already
# gives it that for free on every Linux runner architecture GitHub offers.
$Arch = "x86_64"

$PackageDir = Join-Path $RustDir "target\package"
$StageName = "LaboLabo-rs-windows-$Version-$Arch"
$StageDir = Join-Path $PackageDir $StageName

Write-Host "==> cargo build --release (labolabo-app, labolabo, labolabo-hook), version $Version"
# Same two `-p` flags as bundle-macos.sh/package-linux.sh: `-p labolabo-app`
# builds this package's two bin targets (labolabo-app, the gpui GUI;
# labolabo, the control CLI); `-p labolabo-core` additionally builds its own
# `src/bin/labolabo-hook.rs` (the hooks forwarder).
Push-Location $RustDir
try {
    cargo build --release -p labolabo-app -p labolabo-core
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

$BuildDir = Join-Path $RustDir "target\release"
$Binaries = @("labolabo-app.exe", "labolabo.exe", "labolabo-hook.exe")
foreach ($bin in $Binaries) {
    $binPath = Join-Path $BuildDir $bin
    if (-not (Test-Path $binPath -PathType Leaf)) {
        Write-Error "expected binary not found after build: $binPath"
        exit 1
    }
}

Write-Host "==> Assembling $StageDir (version $Version, arch $Arch)"
if (Test-Path $StageDir) {
    Remove-Item -Recurse -Force $StageDir
}
New-Item -ItemType Directory -Force -Path (Join-Path $StageDir "bin") | Out-Null

# All three binaries live side by side in bin\, same layout convention as
# the macOS bundle's flat Contents/MacOS/ and the Linux tarball's flat
# bin/ -- labolabo-app's hooks integration
# (`crates/labolabo-app/src/hooks.rs`'s `resolve_hook_binary`, EXE_SUFFIX-
# aware since this wave) finds `labolabo-hook.exe` as the sibling of
# `std::env::current_exe()`, so this is load-bearing, not just tidiness.
foreach ($bin in $Binaries) {
    Copy-Item (Join-Path $BuildDir $bin) (Join-Path $StageDir "bin\$bin") -Force
}

# --- Icon --------------------------------------------------------------
#
# A committed `.ico` (`crates/labolabo-app/resources/windows/labolabo-rs.ico`,
# generated from the Swift app's own `icon_512x512@2x.png` artwork -- same
# "must not ship unbranded/placeholder icons" direction bundle-macos.sh's
# icon section documents, regenerate with Pillow:
# `Image.open(png).save(ico, format="ICO", sizes=[(16,16),(24,24),(32,32),
# (48,48),(64,64),(128,128),(256,256)])` -- see that section for why it's
# committed rather than generated here: this repo has no Windows machine in
# its own dev loop to visually verify an embedded-resource `.exe` icon
# against, and GitHub's `windows-latest` runner's ImageMagick availability
# isn't a dependency this script wants to take on for a one-time, easily
# regenerated conversion).
#
# **Not embedded into `labolabo-app.exe`'s own resources** -- deliberately
# the lighter of the two options the task brief allowed ("重ければ zip 内
# .ico 同梱+ショートカット案内に縮退可"): build-time icon embedding
# (`winres`/`embed-resource`, the same crate gpui itself already pulls in
# for its Windows manifest -- see `crates/labolabo-app/README.md`'s
# "Windows" section) is real but adds a Windows-only `build.rs` + RC-compiler
# dependency this repo cannot visually verify the result of (no Windows
# machine in the dev loop); shipping the `.ico` alongside the binaries for a
# user to point their own Start Menu/taskbar shortcut at is lower-risk and
# still gives every user a real, branded icon option. Revisit if/when this
# port gets a proper installer.
$IconSrc = Join-Path $RustDir "crates\labolabo-app\resources\windows\labolabo-rs.ico"
if (-not (Test-Path $IconSrc -PathType Leaf)) {
    Write-Error "Windows icon source not found: $IconSrc"
    exit 1
}
Copy-Item $IconSrc (Join-Path $StageDir "labolabo-rs.ico") -Force

# --- README (zip-local; see crates/labolabo-app/README.md's "Windows"
# section for the full picture -- known limitations, shell resolution) -----
$ReadmeContent = @"
# LaboLabo-rs $Version ($Arch) -- Windows package

## Run

``````
bin\labolabo-app.exe
``````

## Pin a shortcut with the real icon

Windows doesn't offer a root-less "install for this user" convention the
way Linux desktop environments do (see the Linux package's ``install.sh``
in the source repo), so this package ships a bare ``.exe`` + icon instead of
an installer:

1. Right-click ``bin\labolabo-app.exe`` -> **Create shortcut**.
2. Move the shortcut to your Desktop / Start Menu / taskbar as you like.
3. Right-click the shortcut -> **Properties** -> **Change Icon...** -> pick
   ``labolabo-rs.ico`` (next to this README) to use the real app icon
   (the ``.exe`` itself carries no embedded icon in this package -- see
   ``crates/labolabo-app/README.md``'s "Windows" section in the source repo
   for why).

## What's inside

- ``bin\labolabo-app.exe`` -- the gpui terminal-shell GUI.
- ``bin\labolabo.exe`` -- the control CLI (``labolabo tab open``, etc. --
  see ``docs/control-protocol.md`` in the source repo).
- ``bin\labolabo-hook.exe`` -- the Claude Code hooks forwarder; must stay
  next to ``labolabo-app.exe`` (hooks integration finds it as its sibling
  binary).
- ``labolabo-rs.ico`` -- app icon, for a shortcut (see above).

## Known limitations

This build is produced and headless-tested (``cargo build``/``clippy``/
``cargo test``, no window ever opened) by CI on ``windows-latest``; **actual
GUI display on a real Windows desktop has not been verified** -- there is no
Windows machine in this project's own development loop yet. See
``crates/labolabo-app/README.md``'s "Windows" section (in the source repo)
for the full list of known gaps (default shell resolution, Ghostty config
discovery, ConPTY-vs-libghostty differences, IME) if you're building from
source instead of using this prebuilt zip.
"@
Set-Content -Path (Join-Path $StageDir "README.md") -Value $ReadmeContent -NoNewline

# --- zip ---------------------------------------------------------------
$ZipPath = Join-Path $PackageDir "$StageName.zip"
if (Test-Path $ZipPath) {
    Remove-Item -Force $ZipPath
}
Compress-Archive -Path $StageDir -DestinationPath $ZipPath -Force

Write-Host "==> Done"
Write-Host "    Staged: $StageDir"
Write-Host "    Zip: $ZipPath"
