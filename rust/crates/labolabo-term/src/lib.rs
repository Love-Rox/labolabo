//! `labolabo-term`: the cross-platform terminal-session core for LaboLabo.
//!
//! A real PTY (`portable-pty`) drives a VT parser on a background worker
//! thread, which extracts a plain-data [`GridSnapshot`] (cell text, resolved
//! fg/bg RGB, style flags, cursor) and notifies listeners over a wakeup
//! channel. There is **no UI dependency** -- rendering is the future
//! `labolabo-ui`'s job; this crate only produces snapshots.
//!
//! # Backends
//!
//! The VT core is pluggable behind [`backend::VtBackend`]:
//!
//! - **`backend-ghostty-vt`** -- the real `libghostty-vt` engine. **This is
//!   the intended production backend.** Opt-in, because building it needs a
//!   local Ghostty source tree (`GHOSTTY_SOURCE_DIR`) compiled with Zig 0.16.
//! - **`backend-alacritty`** (default) -- an `alacritty_terminal`-based core
//!   that resolves entirely from crates.io, so `cargo test` and CI are always
//!   green with no extra toolchain. A parallel implementation kept honest by
//!   running the *same* integration tests (`tests/backend_common.rs`) as
//!   ghostty.
//!
//! Exactly one backend is active per build. [`ActiveBackend`] resolves to it
//! (preferring ghostty when both features are on), and [`Terminal`] is the
//! ready-to-use session type -- so call sites name no backend:
//!
//! ```no_run
//! use labolabo_term::Terminal;
//! let term = Terminal::spawn_with_command(80, 24, Some("echo hi"), &[]).unwrap();
//! let snap = term.wait_for(std::time::Duration::from_secs(2), |g| g.contains_text("hi"));
//! assert!(snap.is_some());
//! ```
//!
//! Both backends share every OS/PTY/threading concern via [`TermSession`] --
//! only the ~100-line VT-core slice differs. See `session.rs` for the
//! threading model and `backend/mod.rs` for the split.

pub mod backend;
pub mod color;
pub mod session;
pub mod snapshot;

pub use color::ColorScheme;
pub use session::{SharedWriter, TermEvent, TermSession};
pub use snapshot::{CellSnapshot, CursorSnapshot, GridSnapshot, Rgb};

#[cfg(not(any(feature = "backend-alacritty", feature = "backend-ghostty-vt")))]
compile_error!(
    "labolabo-term requires a backend feature: enable `backend-alacritty` (default) \
     or `backend-ghostty-vt`."
);

/// The VT backend selected for this build. Ghostty (the intended production
/// backend) wins when both features are enabled; otherwise alacritty.
#[cfg(feature = "backend-ghostty-vt")]
pub type ActiveBackend = backend::ghostty::GhosttyBackend;

/// The VT backend selected for this build.
#[cfg(all(feature = "backend-alacritty", not(feature = "backend-ghostty-vt")))]
pub type ActiveBackend = backend::alacritty::AlacrittyBackend;

/// A ready-to-use terminal session on the active backend. This is the type
/// callers (and the shared integration tests) should use.
#[cfg(any(feature = "backend-alacritty", feature = "backend-ghostty-vt"))]
pub type Terminal = TermSession<ActiveBackend>;
