//! Backend abstraction: the small, backend-specific slice of a terminal
//! session (the VT core), behind a common trait so the rest of the crate --
//! PTY spawn, the read loop, snapshot throttling, the event channel, the
//! public `TermSession` API -- is written exactly once and shared.
//!
//! **The intended production backend is [`ghostty::GhosttyBackend`]**
//! (real `libghostty-vt`). [`alacritty::AlacrittyBackend`] is a parallel,
//! crates.io-only implementation kept as the *default* purely so a plain
//! `cargo test` and the always-on CI job stay green without a Zig toolchain
//! or a vendored Ghostty checkout. Both are held to the same integration
//! tests (`tests/backend_common.rs`), so the fallback can't silently rot.
//!
//! Only one backend is selected at a time (see `crate::ActiveBackend`), so
//! this is compile-time monomorphization, not runtime dispatch.

use crate::color::ColorScheme;
use crate::session::SharedWriter;
use crate::snapshot::GridSnapshot;

#[cfg(feature = "backend-alacritty")]
pub mod alacritty;
#[cfg(feature = "backend-ghostty-vt")]
pub mod ghostty;

/// A backend's VT core: parse PTY bytes, track a resizable grid, and extract
/// a plain [`GridSnapshot`].
///
/// An implementor lives **entirely on the session's worker thread** -- it is
/// constructed there (inside the spawned thread, never moved across a thread
/// boundary) and every method is called only from that thread. It therefore
/// does **not** need to be `Send`/`Sync`, which matters because
/// `libghostty-vt`'s `Terminal` is neither (its C API requires the caller to
/// serialize all access, which single-thread ownership satisfies for free).
pub trait VtBackend: 'static {
    /// Build a fresh VT core sized `cols` x `rows`.
    ///
    /// `pty_writer` is the shared handle the core writes VT *responses* back
    /// through (device-status reports, cursor-position queries, ...) -- the
    /// replies that full-screen programs (vim, tmux, htop) block on at
    /// startup. Both backends wire this up at construction: alacritty via its
    /// `EventListener::PtyWrite`, ghostty via `Terminal::on_pty_write`.
    ///
    /// `colors` seeds the VT core's default fg/bg/cursor/palette from the
    /// caller's configured [`ColorScheme`] (e.g. the user's own Ghostty
    /// config, as read by `labolabo-app`). Fields left unset in `colors`
    /// keep the backend's own built-in default -- a `ColorScheme::default()`
    /// session renders identically to before this parameter existed.
    fn new(
        cols: u16,
        rows: u16,
        pty_writer: SharedWriter,
        colors: &ColorScheme,
    ) -> anyhow::Result<Self>
    where
        Self: Sized;

    /// Feed raw PTY output bytes into the VT parser.
    fn feed(&mut self, bytes: &[u8]);

    /// Resize the grid. The caller resizes the PTY (kernel `TIOCSWINSZ`)
    /// separately; this only updates the VT core's own dimensions.
    fn resize(&mut self, cols: u16, rows: u16);

    /// Extract the current grid as a plain-data snapshot. `None` means "no
    /// snapshot available this time" (a transient backend error) -- the
    /// worker simply skips publishing rather than tearing down the session.
    fn build_snapshot(&mut self) -> Option<GridSnapshot>;

    /// Whether bracketed paste mode (DECSET `2004`) is currently enabled.
    ///
    /// Queried after every processed PTY byte batch and cached in a plain
    /// `bool` the caller thread can read without blocking (see
    /// `TermSession::bracketed_paste`) -- the same "publish a cheap plain-
    /// data snapshot for the caller thread" shape as [`Self::build_snapshot`],
    /// just for a single flag instead of the whole grid. A pasting caller
    /// (`labolabo-app`'s Cmd+V handler) uses this to decide whether to wrap
    /// the pasted text in `ESC[200~...ESC[201~`.
    fn bracketed_paste(&self) -> bool;
}
