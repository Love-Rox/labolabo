//! `LABOLABO_LOG_OSC52`-gated diagnostic logging for the OSC 52 clipboard
//! pipeline -- a developer escape hatch for narrowing down a future report
//! like the one that motivated it: a long-lived pane's `Cmd+C`/OSC 52 copy
//! silently stops working after several hours, while a brand-new pane in
//! the same app instance still works fine (the `Osc52Scanner` itself was
//! fuzz/soak-tested and found not to be the cause -- see `crate::osc52`'s
//! test module -- so if this recurs, the question is *where* between "the
//! scanner saw it" and "the OS clipboard got it" the payload goes missing,
//! and this log is what answers that without needing a live repro in a
//! debugger).
//!
//! Silent by default (checked once per call, no caching -- matches this
//! codebase's other `LABOLABO_*` debug-toggle env vars, e.g.
//! `labolabo-app`'s `LABOLABO_DEV_FORCE_RUNNING`/`LABOLABO_REDUCE_MOTION`:
//! a plain `std::env::var` read, cheap enough that skipping it isn't worth
//! the complexity of caching, and correct even if a test or a future
//! caller changes the var between calls within one process). Only this
//! crate's half of the pipeline (the worker thread that runs
//! `Osc52Scanner::feed`) is logged here; `labolabo-app`'s
//! `task_workspace::spawn_redraw_bridge` logs its own half (the
//! `take_clipboard_set` → `cx.write_to_clipboard` hop) the same way,
//! reading the same env var independently -- see that function's doc
//! comment. Comparing the two crates' timestamps for the same pane is
//! exactly the point: a healthy pane should show its `write_to_clipboard`
//! line within one `FRAME_INTERVAL`-ish of its `scanner detected` line,
//! every time; a pane that stops copying but keeps producing "detected"
//! lines with no matching "wrote" line would point squarely at the
//! `labolabo-app` half of the hop instead of this scanner.
//!
//! Deliberately never logs payload *content*: only its byte length. A
//! clipboard payload can carry anything the user copied in their terminal
//! (secrets included) -- this log exists to debug a plumbing bug, not to
//! capture clipboard history.

/// Whether `LABOLABO_LOG_OSC52=1` is set in this process's environment --
/// the same one-shot `std::env::var(...).as_deref() == Ok("1")` idiom
/// `labolabo-app`'s `LABOLABO_DEV_FORCE_RUNNING`/`LABOLABO_REDUCE_MOTION`
/// use, deliberately not cached (see module doc comment).
fn enabled() -> bool {
    std::env::var("LABOLABO_LOG_OSC52").as_deref() == Ok("1")
}

/// Milliseconds since the Unix epoch, wall-clock -- not monotonic, but
/// good enough to eyeball-correlate this crate's log lines against
/// `labolabo-app`'s own (see module doc comment), which is this
/// timestamp's only purpose. `0` in the practically-impossible case the
/// system clock reads before the epoch, rather than panicking a worker
/// thread over a debug log line.
fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Logs (to stderr, `LABOLABO_LOG_OSC52=1` only) that
/// [`crate::osc52::Osc52Scanner`] just recognized a complete OSC 52
/// clipboard-set on the session worker thread -- called from inside its
/// `on_clipboard_set` callback in `session::run_worker`, so this fires
/// exactly once per sequence the scanner reports, before the payload is
/// stashed into `TermSession`'s `clipboard` slot for `labolabo-app` to
/// pick up.
///
/// `pane`: the owning pane's `LABOLABO_PANE` uuid, if this session was
/// spawned with one (see `TermSession::spawn_with_scrollback_options`'s
/// `osc52_log_pane` extraction) -- `None` logs as `pane=?` rather than
/// omitting the field, so the log shape stays grep-friendly either way.
/// `payload_len`: the decoded payload's length in bytes -- never the
/// payload itself (see module doc comment).
pub(crate) fn maybe_log_scanner_detected(pane: Option<&str>, payload_len: usize) {
    if !enabled() {
        return;
    }
    eprintln!(
        "labolabo-term: osc52 detected at={} pane={} payload_len={payload_len}",
        now_millis(),
        pane.unwrap_or("?"),
    );
}
