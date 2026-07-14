//! `labolabo-hook <socket_path>`: the Rust standalone equivalent of the
//! Swift app's fused `labolabo --hook <socket>` forwarder mode
//! (`app/Sources/LaboLaboApp.swift` + `app/Sources/HookForwarder.swift`).
//! Deliberately a *separate* binary here (no Rust GUI app exists yet to
//! fuse it into) rather than a `--hook` flag on some future combined
//! binary; `docs/hooks-protocol.md` §2 only specifies the invocation as
//! `'<executable>' --hook '<socketPath>'`, so once a Rust app binary
//! exists it can either keep shelling out to this bin or absorb its logic
//! the way the Swift app does -- not decided here.
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

use std::collections::HashMap;
use std::io::Read;

fn main() {
    let socket_path = std::env::args().nth(1).unwrap_or_default();

    let mut stdin_bytes = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut stdin_bytes);

    let env: HashMap<String, String> = std::env::vars().collect();

    let _ = labolabo_core::forward_hook(&socket_path, &stdin_bytes, &env);

    // Always succeed: a hook's failure must never block Claude Code.
    std::process::exit(0);
}
