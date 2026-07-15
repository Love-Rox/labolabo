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
use crate::mouse::MouseMode;
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
    ///
    /// `max_scrollback` caps how many lines of history the grid retains
    /// past the live viewport (both backends previously hardcoded `1000`
    /// here; `labolabo-app`'s settings screen now makes this user-
    /// configurable -- see `TermSession::spawn_with_scrollback_options`).
    fn new(
        cols: u16,
        rows: u16,
        pty_writer: SharedWriter,
        colors: &ColorScheme,
        max_scrollback: usize,
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

    /// Scroll the viewport by `delta_lines`, relative to wherever it
    /// currently is.
    ///
    /// **Sign convention (shared by both backends and by
    /// [`crate::GridSnapshot::scroll_offset`], chosen to match
    /// `alacritty_terminal`'s own native `Grid::scroll_display(Scroll::
    /// Delta)` convention directly): positive scrolls *up*, into history
    /// (older content becomes visible, `scroll_offset` increases); negative
    /// scrolls *down*, toward the live tail (`scroll_offset` decreases).**
    /// This is also the sign convention real Ghostty's own apprt layer
    /// normalizes trackpad/wheel input to before calling its `Surface.
    /// scrollCallback` (confirmed by reading the vendored Ghostty source's
    /// `Surface.zig`: `ScrollAmount` is documented "negative is down, left
    /// and positive is up, right", and macOS's `SurfaceView_AppKit.swift`
    /// forwards `NSEvent.scrollingDeltaY` to it unmodified -- the same raw
    /// value gpui's own `ScrollWheelEvent.delta` carries on macOS, so
    /// `labolabo-app`'s wheel handler can feed a raw platform delta in here
    /// with no extra sign flip).
    ///
    /// `libghostty-vt`'s own `Terminal::scroll_viewport(ScrollViewport::
    /// Delta)` uses the **opposite** convention (its doc comment: "up is
    /// negative") -- the ghostty backend negates the delta internally so
    /// callers of *this* trait method never need to know that.
    ///
    /// Always clamped internally to `[0, scrollback length]` -- an
    /// out-of-range delta (e.g. `i64::MIN`/`i64::MAX`, or simply "more than
    /// there is history for") saturates rather than panicking or wrapping.
    /// A no-op delta (`0`, or one that clamps to the current offset) is
    /// harmless to call.
    fn scroll_display(&mut self, delta_lines: i64);

    /// Snap the viewport back to the live tail (`scroll_offset` `0`) in one
    /// call, without the caller needing to know the current scrollback
    /// length to compute an equivalent large `scroll_display` delta.
    /// `labolabo-app` calls this on every keystroke that reaches the PTY --
    /// the terminal-UI convention this crate follows (typing while scrolled
    /// back jumps you to the live output, same as every mainstream
    /// terminal).
    fn scroll_to_bottom(&mut self);

    /// Whether the alternate screen buffer (DECSET `1049`/`47`/`1047` -- the
    /// full-screen mode `vim`, `less`, `htop`, and similar TUI programs use)
    /// is currently active.
    ///
    /// `labolabo-app`'s wheel handler uses this to decide whether a
    /// scroll gesture should move *this* crate's own viewport
    /// ([`Self::scroll_display`]) or instead be translated into cursor-key
    /// escape sequences written straight to the PTY -- real Ghostty's
    /// default behavior for alt-screen programs (DECSET `1007`, "alternate
    /// scroll mode", which defaults on: confirmed in the vendored Ghostty
    /// source, `terminal/modes.zig`'s `mouse_alternate_scroll` entry has
    /// `.default = true`), since alt-screen programs typically manage their
    /// own internal scrolling (e.g. `vim`'s buffer, `less`'s pager) rather
    /// than sharing this crate's history buffer -- and indeed the alt
    /// screen has no scrollback of its own on either backend (alacritty:
    /// `Term::new` gives the inactive/alternate grid a `max_scroll_limit` of
    /// `0`; ghostty-vt's alternate screen is likewise not part of the
    /// primary screen's scrollback), so [`Self::scroll_display`] would be a
    /// silent no-op there regardless.
    fn alt_screen_active(&self) -> bool;

    /// Whether "alternate scroll mode" (DECSET `1007`) is currently
    /// active: while [`Self::alt_screen_active`] is also true, this tells
    /// [`crate::TermSession`]'s caller whether a wheel/trackpad scroll
    /// gesture over the alt screen should be translated into cursor-key
    /// escape sequences (`labolabo-app`'s existing behavior for `vim`/
    /// `less`/`htop`-style programs that manage their own internal
    /// scrolling) -- or, when `false`, left alone entirely (neither
    /// forwarded as cursor keys nor scrolled locally -- there is nothing
    /// sensible to do with a scroll gesture over a bare alt-screen program
    /// that has explicitly opted out of both mouse reporting *and*
    /// alternate scroll).
    ///
    /// **Defaults to `true`** on both backends when unset (confirmed by
    /// reading each backend's own source): `alacritty_terminal`'s
    /// `TermMode::default()` includes `ALTERNATE_SCROLL`; `libghostty-vt`'s
    /// underlying Zig source (`terminal/modes.zig`) gives its
    /// `mouse_alternate_scroll` entry `.default = true`. A caller
    /// checking this before mouse-tracking is even considered (see
    /// `labolabo-app`'s wheel handler, which checks [`Self::mouse_mode`]
    /// first and only falls through to this when tracking is off) gets the
    /// same "convert to cursor keys" behavior this crate already had before
    /// this method existed, for every alt-screen program that hasn't
    /// explicitly disabled it -- this method only changes behavior for the
    /// rarer case of a program that both uses the alt screen *and*
    /// explicitly sends `ESC[?1007l`.
    fn alternate_scroll_active(&self) -> bool;

    /// The running program's currently requested mouse-reporting
    /// configuration (DECSET `9`/`1000`/`1002`/`1003` tracking mode, DECSET
    /// `1006` SGR coordinates) -- see [`MouseMode`]'s doc comment.
    ///
    /// Queried after every processed PTY byte batch and cached in a plain
    /// `MouseMode` the caller thread can read without blocking (see
    /// [`crate::TermSession::mouse_mode`]) -- the same "publish a cheap
    /// plain-data flag for the caller thread" shape as
    /// [`Self::bracketed_paste`]/[`Self::alt_screen_active`], just for a
    /// small struct instead of a single bool. `labolabo-app`'s mouse-event
    /// routing uses this to decide whether a click/drag/scroll should be
    /// SGR-encoded and forwarded to the child (vim, tmux, a mouse-aware
    /// TUI, ...) instead of driving this crate's own text-selection/
    /// scrollback UI.
    fn mouse_mode(&self) -> MouseMode;
}
