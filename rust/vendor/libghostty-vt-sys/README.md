# libghostty-vt-sys (LaboLabo vendored copy -- W14 CPU-baseline patch)

This is a local, `[patch.crates-io]`-pinned copy of upstream
[`libghostty-vt-sys` 0.2.0](https://crates.io/crates/libghostty-vt-sys)
(https://github.com/uzaaft/libghostty-rs), with one deliberate change from
the published crate: `build.rs` now always passes `-Dtarget=<triple>` to
`zig build`, even for a native (non-cross) build. See the "LOCAL PATCH"
comment in `build.rs` (`build_vendored`) for the full rationale; short
version: without an explicit `-Dtarget`, Zig auto-detects and bakes in the
*exact* build machine's CPU features (confirmed empirically via
`-femit-llvm-ir` `target-features` dumps -- e.g. AVX-512 on a wide-ISA
runner), which caused intermittent `SIGILL` crashes in CI (GitHub Actions'
runner fleet has heterogeneous CPUs, and `mlugg/setup-zig`'s cross-run
`.zig-cache` reuse could hand a binary built on one CPU to a runner that
lacks the instructions it used) and is a live risk for shipped Linux/macOS
release binaries. An explicit target triple with no CPU suffix resolves to
the portable baseline CPU for that architecture instead.

Everything else in this directory (including `src/bindings.rs`) is the
unmodified published crate source. Update by re-vendoring from
`~/.cargo/registry/src/*/libghostty-vt-sys-<version>/` and re-applying the
same `-Dtarget` fix when bumping the pinned version in `Cargo.lock`. Drop
this patch entirely once the fix (or an equivalent) lands upstream.

---

Raw FFI bindings for libghostty-vt.

- Fetches and builds `libghostty-vt.a` from ghostty sources via Zig by default.
- Exposes checked-in generated bindings in `src/bindings.rs`.
- Static linking is the baseline rather than a Cargo feature. Enable the
  additive `link-dynamic` feature to link the shared library instead.
- Set `GHOSTTY_SOURCE_DIR` to force the build to use a local Ghostty checkout.
- Set `GHOSTTY_ZIG_SYSTEM_DIR` to force Zig package resolution through a
  pre-fetched `zig build --system` directory. This is intended for Nix and other
  sandboxed package managers that cannot fetch during build scripts.
- Set `LIBGHOSTTY_VT_SYS_OPTIMIZE` to `Debug`, `ReleaseSafe`, `ReleaseFast`, or
  `ReleaseSmall` to override the Zig optimize mode used by vendored builds.
- If the `pkg-config` feature is enabled, the build will use an installed
  `libghostty-vt` found through `pkg-config` only when `GHOSTTY_SOURCE_DIR` is
  unset. With the default static link mode, it probes Ghostty's
  `libghostty-vt-static` pkg-config module instead.
- libghostty-vt is pre-1.0, so these bindings do not guarantee compatibility
  with arbitrary installed C API revisions.
