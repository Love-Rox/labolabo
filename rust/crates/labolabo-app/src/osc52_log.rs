//! `LABOLABO_LOG_OSC52`-gated diagnostic logging for this half of the OSC 52
//! clipboard pipeline (`task_workspace::spawn_redraw_bridge`'s
//! `take_clipboard_set` → `cx.write_to_clipboard` hop). Pairs with
//! `labolabo_term::osc52_log`'s worker-thread-side logging (the scanner
//! detection itself) -- see that module's doc comment for the full
//! rationale (a long-lived pane's OSC 52 copy silently stopping while new
//! panes keep working) and how the two crates' log lines are meant to be
//! read together.
//!
//! Same env var (`LABOLABO_LOG_OSC52=1`), read independently here (no
//! shared state with `labolabo_term`'s copy of the check -- this crate has
//! no dependency on that one's private internals, and there's no shared
//! logging module anywhere in this codebase to route through instead).
//! Same one-shot, uncached `std::env::var` idiom as this crate's other
//! `LABOLABO_*` debug toggles (`LaboLaboApp::dev_force_running_if_
//! requested`'s `LABOLABO_DEV_FORCE_RUNNING`, `motion::reduce_motion`'s
//! `LABOLABO_REDUCE_MOTION`). Silent by default; never logs payload
//! *content*, only its byte length -- a clipboard payload can carry
//! anything the user copied (secrets included), and this log exists to
//! debug plumbing, not to capture clipboard history.

use labolabo_core::PaneId;

fn enabled() -> bool {
    std::env::var("LABOLABO_LOG_OSC52").as_deref() == Ok("1")
}

/// Wall-clock milliseconds since the Unix epoch -- see `labolabo_term::
/// osc52_log::now_millis`'s doc comment; this is the same idiom,
/// duplicated rather than shared since the two crates have no logging
/// module in common to put it in.
fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Logs (to stderr, `LABOLABO_LOG_OSC52=1` only) that `spawn_redraw_
/// bridge`'s async loop just pulled a decoded OSC 52 payload off
/// `Terminal::take_clipboard_set` and is about to hand it to
/// `cx.write_to_clipboard` -- called right *before* that call (so the log
/// line's wording is deliberately present-progressive, not "wrote": this
/// fires regardless of whether the subsequent `cx.write_to_clipboard`
/// call actually succeeds). A missing line for a pane that keeps
/// producing `labolabo-term`'s "detected" lines points at this hop (or
/// `write_to_clipboard`/gpui itself) rather than the scanner.
pub(crate) fn maybe_log_app_writing_to_clipboard(
    task_id: &str,
    pane_id: PaneId,
    payload_len: usize,
) {
    if !enabled() {
        return;
    }
    eprintln!(
        "labolabo-app: osc52 writing to clipboard at={} task={task_id} pane={pane_id:?} payload_len={payload_len}",
        now_millis(),
    );
}
