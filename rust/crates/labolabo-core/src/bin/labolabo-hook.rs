//! `labolabo-hook [--hook] <socket_path>`: the Rust standalone equivalent
//! of the Swift app's fused `labolabo --hook <socket>` forwarder mode
//! (`app/Sources/LaboLaboApp.swift` + `app/Sources/HookForwarder.swift`).
//! Deliberately a *separate* binary here (no Rust GUI app exists yet to
//! fuse it into) rather than a `--hook` flag on some future combined
//! binary.
//!
//! Both invocation forms are accepted: `docs/hooks-protocol.md` §2
//! specifies `'<executable>' --hook '<socketPath>'`, and that is exactly
//! what `hook_settings::hook_command` writes into
//! `.claude/settings.local.json` -- so the `--hook` flag form is the one
//! Claude Code actually runs. The original wave-4b positional form
//! (`labolabo-hook <socket>`) stays supported for compatibility. Getting
//! this wrong is silently fatal end to end: this binary exits 0 on *every*
//! failure (see below), so a mis-parsed argv would just drop every hook
//! event on the floor with no error anywhere -- which is precisely what
//! happened when wave 5c first wired the §2 command string up to the
//! then-positional-only argv parsing (caught on-device: no status dots, no
//! session recording, no resume). `tests/labolabo_hook_bin.rs` now pins
//! both forms, including one that runs the exact `hook_command` string
//! through a real `sh -c`.
//!
//! All the logic lives in [`labolabo_core::forward_hook`] (and the
//! `annotate_pane` step it calls internally); this `main` is intentionally
//! thin:
//!
//! 1. Read the socket path from argv\[1\].
//! 2. Read stdin to EOF (the hook event JSON, per
//!    docs/hooks-protocol.md §3.1).
//! 3. Snapshot the process environment (for the `LABOLABO_PANE` ->
//!    `labolabo_pane_id` annotation, §3.2/§7).
//! 4. Call `forward_hook`, ignore its result, and **always** exit 0 --
//!    docs/hooks-protocol.md §3.3: "接続失敗・パス過長などあらゆる失敗も
//!    exit(0)（hook の失敗で Claude を止めない）", matching Swift's
//!    `HookForwarder.forward`, which calls `exit(0)` on every branch.
//!
//! `labolabo_core::forward_hook` is `#[cfg(any(unix, windows))]` -- on unix
//! it connects to the AF_UNIX socket, on Windows to the
//! `\\.\pipe\labolabo-<10hex>` Named Pipe (docs/hooks-protocol.md §4.2;
//! either way `socket_path` is just the string Claude Code passed on the
//! command line, so this `main` needs no per-OS logic). Step 4 stays split
//! behind a small `forward` shim only so any *other* target still compiles
//! as a no-op stub. Every arm exits 0 regardless, preserving the "hook の
//! 失敗で Claude を止めない" contract on every platform.

use std::collections::HashMap;
use std::io::Read;

fn main() {
    // `labolabo-hook --hook <socket>` (docs §2, what hook_command emits) or
    // `labolabo-hook <socket>` (wave-4b positional form).
    let mut args = std::env::args().skip(1);
    let first = args.next().unwrap_or_default();
    let socket_path = if first == "--hook" {
        args.next().unwrap_or_default()
    } else {
        first
    };

    let mut stdin_bytes = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut stdin_bytes);

    let env: HashMap<String, String> = std::env::vars().collect();

    forward(&socket_path, &stdin_bytes, &env);

    // Always succeed: a hook's failure must never block Claude Code.
    std::process::exit(0);
}

#[cfg(any(unix, windows))]
fn forward(socket_path: &str, stdin_bytes: &[u8], env: &HashMap<String, String>) {
    let _ = labolabo_core::forward_hook(socket_path, stdin_bytes, env);
}

#[cfg(not(any(unix, windows)))]
fn forward(_socket_path: &str, _stdin_bytes: &[u8], _env: &HashMap<String, String>) {
    eprintln!("labolabo-hook: forwarding is not implemented on this platform yet");
}
