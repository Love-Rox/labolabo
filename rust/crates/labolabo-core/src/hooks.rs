//! Faithful port of the hooks bus + forwarder wire protocol described in
//! full by `docs/hooks-protocol.md` (checked in at the repo root -- that
//! document is the canonical spec for this module) and implemented in
//! Swift by `Sources/LaboLaboEngine/Agent/AgentStatusBus.swift` (transport
//! contract + AF_UNIX transport + bus) and `app/Sources/HookForwarder.swift`
//! (forwarder). Cross-checked against both directly; no divergence found.
//!
//! Four pieces, mirroring the Swift split:
//!
//! 1. [`AgentEventTransport`]: the transport contract (`onMessage`
//!    callback + `start`/`stop`), OS-independent. The calling thread for
//!    the callback is implementation-defined -- see its doc comment.
//! 2. [`UnixSocketEventTransport`] (`#[cfg(unix)]`): the AF_UNIX
//!    (`SOCK_STREAM`) implementation used on macOS/Linux. A Windows
//!    transport (Named Pipe, per docs/hooks-protocol.md §4) is future work
//!    and has no stub here yet -- only this comment.
//! 3. [`AgentStatusBus`]: composes a transport with `agent_event_parser` to
//!    turn raw bytes into [`crate::AgentStatusEvent`]s. Unlike the Swift
//!    version, this does **not** hop to a main-thread queue itself
//!    (`DispatchQueue.main.async` there is a UI-layer concern with no
//!    analog in this OS/UI-independent core) -- the `on_event` callback set
//!    via [`AgentStatusBus::set_on_event`] is invoked directly on whatever
//!    thread the transport's `on_message` fires on. Marshaling to a UI
//!    thread, if one exists, is the caller's responsibility.
//! 4. [`forward_hook`] (`#[cfg(unix)]`): the pure-ish forwarder logic used
//!    by the thin `labolabo-hook` bin (`src/bin/labolabo-hook.rs`) --
//!    reads are already done by the caller (stdin bytes and the process
//!    environment are passed in explicitly, not read from ambient global
//!    state), so this function is deterministic given its inputs even
//!    though it performs real socket I/O (connect + write + close).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::agent_event_parser;
use crate::AgentStatusEvent;

/// A received raw message (one hook event's payload, before JSON
/// interpretation).
pub type OnMessage = Box<dyn Fn(Vec<u8>) + Send + Sync + 'static>;

/// A parsed [`AgentStatusEvent`] ready for the UI/consumer layer.
pub type OnEvent = Box<dyn Fn(AgentStatusEvent) + Send + Sync + 'static>;

/// The contract a byte-transport for hook events must satisfy. Mirrors the
/// Swift `AgentEventTransport` protocol
/// (`Sources/LaboLaboEngine/Agent/AgentStatusBus.swift`): a settable
/// `onMessage` callback plus `start`/`stop`. `onMessage` must be registered
/// via [`set_on_message`](AgentEventTransport::set_on_message) *before*
/// [`start`](AgentEventTransport::start) is called.
///
/// The thread that invokes the registered callback is
/// implementation-defined (the Swift doc comment says the same: "呼び出し
/// スレッドは実装依存 -- 受信側でキュー移送すること"). Callers that need a
/// specific thread (e.g. a UI main thread) must marshal for themselves.
pub trait AgentEventTransport: Send {
    /// Registers the callback invoked for each received message. Must be
    /// called before `start()`.
    fn set_on_message(&mut self, callback: OnMessage);
    fn start(&mut self);
    fn stop(&mut self);
}

/// Per-session hook-event receive bus. Composes a transport (AF_UNIX by
/// default on unix) with `agent_event_parser::parse` and invokes `on_event`
/// for every successfully-parsed event.
///
/// Faithful port of the Swift `AgentStatusBus` class, minus the
/// `DispatchQueue.main.async` hop -- see the module doc comment.
pub struct AgentStatusBus {
    socket_path: String,
    on_event: Arc<Mutex<Option<OnEvent>>>,
    transport: Box<dyn AgentEventTransport>,
}

impl AgentStatusBus {
    /// Creates a bus with an injected transport. Used by tests (a mock
    /// transport) and by future non-unix platforms (a Named Pipe/TCP
    /// transport) -- mirrors the Swift initializer's `transport:
    /// AgentEventTransport? = nil` injection point.
    pub fn with_transport(
        socket_path: impl Into<String>,
        transport: Box<dyn AgentEventTransport>,
    ) -> Self {
        Self {
            socket_path: socket_path.into(),
            on_event: Arc::new(Mutex::new(None)),
            transport,
        }
    }

    pub fn socket_path(&self) -> &str {
        &self.socket_path
    }

    /// Registers the callback invoked (on a transport-defined thread, see
    /// the module doc comment) for every parsed event. Mirrors Swift's
    /// plain settable `var onEvent`: re-registering after
    /// [`start`](Self::start) takes effect for subsequently-parsed events
    /// too (read fresh per event, not snapshotted at `start()`), but
    /// registering before `start()` is the normal usage -- an event parsed
    /// before a callback is registered is simply not delivered to anyone.
    pub fn set_on_event(&self, callback: impl Fn(AgentStatusEvent) + Send + Sync + 'static) {
        *self.on_event.lock().unwrap() = Some(Box::new(callback));
    }

    /// Wires the transport's raw-byte callback through the parser and
    /// starts the transport.
    pub fn start(&mut self) {
        let on_event = Arc::clone(&self.on_event);
        self.transport.set_on_message(Box::new(move |data| {
            let Some(event) = agent_event_parser::parse(&data) else {
                return;
            };
            if let Some(cb) = on_event.lock().unwrap().as_ref() {
                cb(event);
            }
        }));
        self.transport.start();
    }

    pub fn stop(&mut self) {
        self.transport.stop();
    }
}

#[cfg(unix)]
mod unix_transport {
    use super::{AgentEventTransport, OnMessage};
    use crate::AgentStatusBus;
    use std::fs;
    use std::io::Read;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;

    impl AgentStatusBus {
        /// Creates a bus using the default AF_UNIX transport, matching the
        /// Swift `AgentStatusBus.init(socketPath:)` convenience.
        pub fn new(socket_path: impl Into<String>) -> Self {
            let socket_path = socket_path.into();
            let transport = UnixSocketEventTransport::new(socket_path.clone());
            Self::with_transport(socket_path, Box::new(transport))
        }
    }

    /// Per-session AF_UNIX (`SOCK_STREAM`) socket server. Faithful port of
    /// `Sources/LaboLaboEngine/Agent/AgentStatusBus.swift`'s
    /// `UnixSocketEventTransport`: `labolabo-hook <socket>` connects once
    /// per hook event and sends the whole JSON payload before closing (1
    /// connection = 1 event, docs/hooks-protocol.md §4); the accept loop
    /// reads each connection to EOF and hands the accumulated bytes to
    /// `onMessage`.
    ///
    /// Differences from the Swift implementation are deliberate
    /// robustness improvements to non-load-bearing plumbing, not wire- or
    /// observable-behavior changes:
    /// - reads use `Read::read_to_end` (loops until EOF/error, keeping any
    ///   partial bytes already read on error) instead of Swift's hand-rolled
    ///   `read()` loop -- same "read to EOF, keep whatever arrived" contract;
    /// - `stop()` only calls `shutdown(2)` on the accept thread's listening
    ///   fd to unblock a blocked `accept()`, and lets that thread's own
    ///   `UnixListener` `Drop` perform the single `close(2)` once it wakes
    ///   up and exits its loop -- Swift's `stop()` additionally calls
    ///   `close(fd)` itself, then `runServer()` closes the same fd number
    ///   again after the loop exits, a harmless (EBADF-only) double close
    ///   this port avoids by construction.
    pub struct UnixSocketEventTransport {
        socket_path: String,
        inner: Arc<Inner>,
    }

    struct Inner {
        socket_path: PathBuf,
        on_message: Mutex<Option<OnMessage>>,
        running: AtomicBool,
        started_once: AtomicBool,
        /// Raw fd of the bound listener while the accept loop is live, or
        /// -1 otherwise. Shared so `stop()` (possibly called from a
        /// different thread) can `shutdown(2)` it to unblock a blocked
        /// `accept()`.
        listen_fd: AtomicI32,
    }

    impl UnixSocketEventTransport {
        pub fn new(socket_path: impl Into<String>) -> Self {
            let socket_path = socket_path.into();
            Self {
                socket_path: socket_path.clone(),
                inner: Arc::new(Inner {
                    socket_path: PathBuf::from(socket_path),
                    on_message: Mutex::new(None),
                    running: AtomicBool::new(false),
                    started_once: AtomicBool::new(false),
                    listen_fd: AtomicI32::new(-1),
                }),
            }
        }

        pub fn socket_path(&self) -> &str {
            &self.socket_path
        }
    }

    impl AgentEventTransport for UnixSocketEventTransport {
        fn set_on_message(&mut self, callback: OnMessage) {
            *self.inner.on_message.lock().unwrap() = Some(callback);
        }

        fn start(&mut self) {
            // A second `start()` would race two `run_server` loops over the
            // same socket path and leak a thread/fd -- restrict to once,
            // like the Swift `startedOnce` guard (no restart after `stop`).
            if self.inner.started_once.swap(true, Ordering::SeqCst) {
                return;
            }
            let inner = Arc::clone(&self.inner);
            // `accept()`/`read()` block, so this gets a dedicated thread
            // rather than a shared worker pool (one stays parked per
            // session for the process lifetime) -- same reasoning as the
            // Swift `Thread` (never a `DispatchQueue`) for this loop.
            let _ = thread::Builder::new()
                .name("labolabo.agent.statusbus".to_string())
                .spawn(move || run_server(&inner));
        }

        fn stop(&mut self) {
            self.inner.running.store(false, Ordering::SeqCst);
            let fd = self.inner.listen_fd.swap(-1, Ordering::SeqCst);
            if fd >= 0 {
                // SAFETY: `fd` was obtained from `UnixListener::as_raw_fd()`
                // on the still-live listener owned by the accept-loop
                // thread; `shutdown(2)` on a valid fd is safe and merely
                // unblocks that thread's blocked `accept()` call. This
                // thread does not close the fd itself (see the struct doc
                // comment) -- the owning thread's `UnixListener` closes it
                // exactly once when it drops.
                unsafe {
                    libc::shutdown(fd, libc::SHUT_RDWR);
                }
            }
            let _ = fs::remove_file(&self.inner.socket_path);
        }
    }

    fn run_server(inner: &Arc<Inner>) {
        // Clean up a stale socket file from a previous run before binding
        // (docs/hooks-protocol.md §4: "同一パスはアプリ再起動を跨いで再利用
        // される（起動時に残骸を unlink してから bind）").
        let _ = fs::remove_file(&inner.socket_path);

        // Best-effort: ensure the parent directory exists and is
        // owner-only (docs/hooks-protocol.md §4/§8: "ディレクトリ
        // /tmp/labolabo は 0700 で作成"). Only newly-created directories in
        // the chain get this mode -- an already-existing parent keeps
        // whatever permissions it has, matching `mkdir`/Swift
        // `FileManager.createDirectory(attributes:)` semantics (the
        // attribute only applies at creation time).
        if let Some(parent) = inner.socket_path.parent() {
            let _ = fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent);
        }

        let listener = match UnixListener::bind(&inner.socket_path) {
            Ok(listener) => listener,
            Err(_) => return,
        };

        // Only the owning user may connect (docs/hooks-protocol.md §4/§8).
        let _ = fs::set_permissions(&inner.socket_path, fs::Permissions::from_mode(0o600));

        inner
            .listen_fd
            .store(listener.as_raw_fd(), Ordering::SeqCst);
        inner.running.store(true, Ordering::SeqCst);

        while inner.running.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _addr)) => handle_client(inner, stream),
                Err(_) => {
                    if inner.running.load(Ordering::SeqCst) {
                        continue;
                    } else {
                        break;
                    }
                }
            }
        }

        inner.listen_fd.store(-1, Ordering::SeqCst);
        drop(listener);
        let _ = fs::remove_file(&inner.socket_path);
    }

    fn handle_client(inner: &Inner, mut stream: UnixStream) {
        let mut data = Vec::new();
        // Ignore read errors: `read_to_end` keeps any bytes already read in
        // `data` even when it returns `Err`, matching the Swift loop's
        // break-on-`n<=0` (no distinction between EOF and a read error --
        // whatever arrived before the break is still delivered).
        let _ = stream.read_to_end(&mut data);
        if data.is_empty() {
            return;
        }
        if let Some(cb) = inner.on_message.lock().unwrap().as_ref() {
            cb(data);
        }
    }
}

#[cfg(unix)]
pub use unix_transport::UnixSocketEventTransport;

/// Sends one hook event to the LaboLabo instance listening on
/// `socket_path`: annotates `stdin_bytes` with `labolabo_pane_id`/
/// `labolabo_task_id` (from `env["LABOLABO_PANE"]`/`env["LABOLABO_TASK"]`)
/// when applicable, then connects, writes the whole payload, and closes.
/// Faithful port of `app/Sources/HookForwarder.swift`'s `forward` (minus the
/// `exit(0)` call itself, which is the caller's job -- see
/// `src/bin/labolabo-hook.rs`, whose `main` always exits 0 regardless of
/// this function's result, matching Swift's "hook の失敗で Claude を止めない"
/// contract from docs/hooks-protocol.md §3).
///
/// `env` is taken as an explicit map (not read from the real process
/// environment) so this function is deterministic given its inputs and
/// testable without mutating global process state.
///
/// `labolabo_task_id` (docs/hooks-protocol.md §7: reserved for a future
/// work/task model) has no Swift counterpart -- `LABOLABO_TASK` is
/// Rust-only (`plans/012` §1's Task model), so this annotation step is new
/// here rather than a port of `HookForwarder.swift`.
#[cfg(unix)]
pub fn forward_hook(
    socket_path: &str,
    stdin_bytes: &[u8],
    env: &HashMap<String, String>,
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let payload = annotate_ids(stdin_bytes, env);
    let mut stream = UnixStream::connect(socket_path)?;
    stream.write_all(&payload)?;
    // `stream` closes on drop at the end of this function (write -> close).
    Ok(())
}

/// The `LABOLABO_PANE`/`LABOLABO_TASK` -> `labolabo_pane_id`/
/// `labolabo_task_id` annotation step of the forwarder contract
/// (docs/hooks-protocol.md §3.2 for the pane half; §7 for the task half):
/// if `stdin_bytes` parses as a JSON object, adds/overwrites a top-level
/// `"labolabo_pane_id"` string field for a non-empty `env["LABOLABO_PANE"]`
/// and/or a `"labolabo_task_id"` string field for a non-empty
/// `env["LABOLABO_TASK"]` (independently -- either, both, or neither may be
/// present) and re-serializes. Otherwise (neither env var set, invalid
/// JSON, or a non-object JSON top level) returns `stdin_bytes` unchanged --
/// mirrors Swift's `guard let paneID = ..., var object = (try?
/// JSONSerialization...) as? [String: Any] else { return input }`, extended
/// with the task id (which has no Swift counterpart -- see [`forward_hook`]'s
/// doc comment).
///
/// Its only production caller ([`forward_hook`]) is `#[cfg(unix)]`, but the
/// pure-function tests in `mod tests` below exercise it directly on every
/// platform (no socket I/O needed) -- so it stays compiled everywhere, and
/// the `dead_code` lint that would otherwise fire in a non-test, non-unix
/// build (no caller reachable there) is silenced explicitly rather than by
/// accident.
#[cfg_attr(not(unix), allow(dead_code))]
fn annotate_ids(stdin_bytes: &[u8], env: &HashMap<String, String>) -> Vec<u8> {
    let pane_id = env.get("LABOLABO_PANE").filter(|v| !v.is_empty());
    let task_id = env.get("LABOLABO_TASK").filter(|v| !v.is_empty());
    if pane_id.is_none() && task_id.is_none() {
        return stdin_bytes.to_vec();
    }
    let Ok(Value::Object(mut object)) = serde_json::from_slice::<Value>(stdin_bytes) else {
        return stdin_bytes.to_vec();
    };
    if let Some(pane_id) = pane_id {
        object.insert(
            "labolabo_pane_id".to_string(),
            Value::String(pane_id.clone()),
        );
    }
    if let Some(task_id) = task_id {
        object.insert(
            "labolabo_task_id".to_string(),
            Value::String(task_id.clone()),
        );
    }
    serde_json::to_vec(&Value::Object(object)).unwrap_or_else(|_| stdin_bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentStatus;

    // MARK: - annotate_ids (forwarder pane/task-id annotation)
    //
    // The pane-id cases are not ported from a Swift XCTest
    // (HookForwarder.swift has none in the Swift codebase) -- these are the
    // three scenarios named in the wave 4b porting brief: LABOLABO_PANE
    // present, absent, and non-JSON stdin. The task-id cases are new
    // (LABOLABO_TASK/labolabo_task_id has no Swift counterpart).

    fn env_with_pane(value: &str) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert("LABOLABO_PANE".to_string(), value.to_string());
        env
    }

    fn env_with_task(value: &str) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert("LABOLABO_TASK".to_string(), value.to_string());
        env
    }

    #[test]
    fn annotate_pane_adds_field_when_pane_env_present_and_stdin_is_json_object() {
        let env = env_with_pane("PANE-1");
        let out = annotate_ids(br#"{"hook_event_name":"SessionStart"}"#, &env);
        let value: Value = serde_json::from_slice(&out).expect("valid json");
        assert_eq!(value["hook_event_name"], "SessionStart");
        assert_eq!(value["labolabo_pane_id"], "PANE-1");
        assert!(value.get("labolabo_task_id").is_none());
    }

    #[test]
    fn annotate_pane_passes_through_unchanged_when_pane_env_absent() {
        let env = HashMap::new();
        let input = br#"{"hook_event_name":"Stop"}"#;
        assert_eq!(annotate_ids(input, &env), input.to_vec());
    }

    #[test]
    fn annotate_pane_passes_through_unchanged_when_stdin_is_not_a_json_object() {
        let env = env_with_pane("PANE-2");
        // Malformed JSON.
        let malformed: &[u8] = b"{ not json";
        assert_eq!(annotate_ids(malformed, &env), malformed.to_vec());
        // Syntactically valid JSON, but not an object (Swift's `as? [String:
        // Any]` cast also fails for this, matching `agent_event_parser`'s
        // `non_object_top_level_is_dropped` quirk).
        let array: &[u8] = b"[1,2,3]";
        assert_eq!(annotate_ids(array, &env), array.to_vec());
    }

    #[test]
    fn annotate_pane_empty_pane_env_is_treated_as_absent() {
        // Swift guards on `!paneID.isEmpty`; an empty LABOLABO_PANE must not
        // annotate either.
        let env = env_with_pane("");
        let input = br#"{"hook_event_name":"Stop"}"#;
        assert_eq!(annotate_ids(input, &env), input.to_vec());
    }

    #[test]
    fn annotate_task_adds_field_when_task_env_present_and_stdin_is_json_object() {
        let env = env_with_task("TASK-1");
        let out = annotate_ids(br#"{"hook_event_name":"SessionStart"}"#, &env);
        let value: Value = serde_json::from_slice(&out).expect("valid json");
        assert_eq!(value["labolabo_task_id"], "TASK-1");
        assert!(value.get("labolabo_pane_id").is_none());
    }

    #[test]
    fn annotate_task_empty_task_env_is_treated_as_absent() {
        let env = env_with_task("");
        let input = br#"{"hook_event_name":"Stop"}"#;
        assert_eq!(annotate_ids(input, &env), input.to_vec());
    }

    #[test]
    fn annotate_ids_adds_both_fields_when_both_env_vars_present() {
        let mut env = env_with_pane("PANE-3");
        env.insert("LABOLABO_TASK".to_string(), "TASK-3".to_string());
        let out = annotate_ids(br#"{"hook_event_name":"SessionStart"}"#, &env);
        let value: Value = serde_json::from_slice(&out).expect("valid json");
        assert_eq!(value["labolabo_pane_id"], "PANE-3");
        assert_eq!(value["labolabo_task_id"], "TASK-3");
    }

    // MARK: - AgentStatusBus transport injection contract
    //
    // Not ported from Swift 1:1 (the Swift `AgentStatusBusTests` always
    // exercises the real `UnixSocketEventTransport` over an actual socket --
    // see the `unix_bus_tests` module below for that port). This test is new:
    // it proves the composition (`AgentEventTransport::onMessage` ->
    // `agent_event_parser::parse` -> `AgentStatusBus::on_event`) in isolation,
    // using a hand-rolled mock transport instead of a real socket, exercising
    // exactly the DI seam `AgentStatusBus::with_transport` exists for (mirrors
    // the Swift initializer's `transport: AgentEventTransport? = nil` param).

    struct MockTransport {
        on_message_slot: Arc<Mutex<Option<OnMessage>>>,
        start_calls: Arc<std::sync::atomic::AtomicUsize>,
        stop_calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl AgentEventTransport for MockTransport {
        fn set_on_message(&mut self, callback: OnMessage) {
            *self.on_message_slot.lock().unwrap() = Some(callback);
        }

        fn start(&mut self) {
            self.start_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        fn stop(&mut self) {
            self.stop_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn mock_transport_injection_wires_parser_and_dispatches_events() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let on_message_slot: Arc<Mutex<Option<OnMessage>>> = Arc::new(Mutex::new(None));
        let start_calls = Arc::new(AtomicUsize::new(0));
        let stop_calls = Arc::new(AtomicUsize::new(0));
        let transport = MockTransport {
            on_message_slot: Arc::clone(&on_message_slot),
            start_calls: Arc::clone(&start_calls),
            stop_calls: Arc::clone(&stop_calls),
        };

        let mut bus = AgentStatusBus::with_transport("mock-socket-path", Box::new(transport));

        let received: Arc<Mutex<Vec<AgentStatusEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let received_for_callback = Arc::clone(&received);
        bus.set_on_event(move |event| received_for_callback.lock().unwrap().push(event));

        bus.start();
        assert_eq!(
            start_calls.load(Ordering::SeqCst),
            1,
            "AgentStatusBus::start() should call transport.start() exactly once"
        );

        // Simulate the transport receiving a raw byte payload -- no real
        // socket involved, this is purely testing that `start()` wired
        // `onMessage` through the parser to `on_event`.
        let callback = on_message_slot
            .lock()
            .unwrap()
            .take()
            .expect("bus.start() should have registered onMessage on the transport");
        callback(br#"{"hook_event_name":"SessionStart","session_id":"mock-1"}"#.to_vec());

        {
            let events = received.lock().unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].status, AgentStatus::Starting);
            assert_eq!(events[0].session_id.as_deref(), Some("mock-1"));
        }

        // An unknown hook event is dropped by the parser before it ever
        // reaches `on_event` -- same drop contract as the real-socket tests.
        callback(br#"{"hook_event_name":"Mystery"}"#.to_vec());
        assert_eq!(
            received.lock().unwrap().len(),
            1,
            "unrecognized hook_event_name must not dispatch a second event"
        );

        bus.stop();
        assert_eq!(
            stop_calls.load(Ordering::SeqCst),
            1,
            "AgentStatusBus::stop() should call transport.stop() exactly once"
        );
    }
}

/// Real AF_UNIX socket round-trip tests, ported 1:1 from
/// `Tests/LaboLaboEngineTests/AgentStatusBusTests.swift` (6 tests): a real
/// POSIX client connects to `AgentStatusBus`'s socket and sends one payload
/// per connection (mirrors the forwarder's "1 connection = 1 event"
/// contract), and `on_event` is asserted to fire (or not fire) with the
/// right `AgentStatusEvent`.
#[cfg(all(test, unix))]
mod unix_bus_tests {
    use super::*;
    use crate::AgentStatus;
    use std::fs;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// Short, likely-unique socket path under the OS temp dir -- mirrors the
    /// Swift test's `UUID().uuidString.prefix(8)` reasoning: `sockaddr_un`'s
    /// `sun_path` is only 104 (Darwin) / 108 (Linux) bytes, so the file name
    /// must stay short.
    fn temp_socket_path() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let short = format!("{:x}{:x}", nanos as u32, n);
        std::env::temp_dir().join(format!("lb-{short}.sock"))
    }

    /// Connects an AF_UNIX/SOCK_STREAM client and sends `payload`, then
    /// half-closes the write side so the server's read loop sees EOF.
    /// Retries the connect (the accept-loop thread needs a moment to bind)
    /// like the Swift test's `sendPayload`.
    fn send_payload(path: &std::path::Path, payload: &[u8]) -> bool {
        for _ in 0..150 {
            match UnixStream::connect(path) {
                Ok(mut stream) => {
                    let ok = payload.is_empty() || stream.write_all(payload).is_ok();
                    let _ = stream.shutdown(std::net::Shutdown::Write);
                    return ok;
                }
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
        false
    }

    /// Starts a bus on a fresh temp socket, sends `payload`, and waits (up
    /// to 3s) for `on_event` to fire.
    fn expect_event(payload: &[u8]) -> Option<AgentStatusEvent> {
        let path = temp_socket_path();
        let mut bus = AgentStatusBus::new(path.to_str().expect("utf8 path"));
        let (tx, rx) = mpsc::channel();
        bus.set_on_event(move |event| {
            let _ = tx.send(event);
        });
        bus.start();

        assert!(
            send_payload(&path, payload),
            "client should be able to send the payload"
        );
        let event = rx.recv_timeout(Duration::from_secs(3)).ok();

        bus.stop();
        let _ = fs::remove_file(&path);
        event
    }

    /// Starts a bus, sends `payload`, and asserts `on_event` does *not* fire
    /// within 1s.
    fn expect_no_event(payload: &[u8]) {
        let path = temp_socket_path();
        let mut bus = AgentStatusBus::new(path.to_str().expect("utf8 path"));
        let (tx, rx) = mpsc::channel::<AgentStatusEvent>();
        bus.set_on_event(move |event| {
            let _ = tx.send(event);
        });
        bus.start();

        assert!(
            send_payload(&path, payload),
            "client should be able to send the payload"
        );
        let result = rx.recv_timeout(Duration::from_secs(1));

        bus.stop();
        let _ = fs::remove_file(&path);
        assert!(
            result.is_err(),
            "onEvent should not have fired, but got {result:?}"
        );
    }

    // MARK: - 正常系（hook_event_name → AgentStatus のマッピング）

    #[test]
    fn notification_round_trip_emits_waiting_for_input() {
        let json = br#"{"hook_event_name":"Notification","session_id":"s1","transcript_path":"/tmp/t.jsonl","cwd":"/tmp"}"#;
        let event = expect_event(json).expect("bus should emit one event");
        assert_eq!(event.status, AgentStatus::WaitingForInput);
        assert_eq!(event.hook_event, "Notification");
        assert_eq!(event.session_id.as_deref(), Some("s1"));
        assert_eq!(event.transcript_path.as_deref(), Some("/tmp/t.jsonl"));
        assert_eq!(event.cwd.as_deref(), Some("/tmp"));
        // フォワーダ由来の pane id が無い（外部ターミナル等）場合は None。
        assert_eq!(event.pane_id, None);
    }

    #[test]
    fn pane_id_is_parsed_when_forwarder_annotates() {
        // フォワーダが LABOLABO_PANE から付与する labolabo_pane_id がイベン
        // トへ載ること。タブ別 resume（session_id ↔ ペインの対応付け）の要。
        let json =
            br#"{"hook_event_name":"SessionStart","session_id":"s9","labolabo_pane_id":"ABC-123"}"#;
        let event = expect_event(json).expect("bus should emit one event");
        assert_eq!(event.status, AgentStatus::Starting);
        assert_eq!(event.session_id.as_deref(), Some("s9"));
        assert_eq!(event.pane_id.as_deref(), Some("ABC-123"));
    }

    #[test]
    fn stop_event_round_trip_emits_idle() {
        // 別イベント種別が別 status にマップされること（Stop → .idle）も
        // 1 本で押さえる。
        let json = br#"{"hook_event_name":"Stop","session_id":"s2","transcript_path":"/tmp/s2.jsonl","cwd":"/work"}"#;
        let event = expect_event(json).expect("bus should emit one event");
        assert_eq!(event.status, AgentStatus::Idle);
        assert_eq!(event.hook_event, "Stop");
        assert_eq!(event.session_id.as_deref(), Some("s2"));
        assert_eq!(event.cwd.as_deref(), Some("/work"));
    }

    // MARK: - 異常系（無イベント）

    #[test]
    fn malformed_json_produces_no_event() {
        // 接続・書き込みは成功するが JSON として壊れている → parse 段でド
        // ロップ。
        expect_no_event(b"{ this is not valid json ");
    }

    #[test]
    fn empty_payload_produces_no_event() {
        // 接続して即クローズ（0 バイト）→ data.is_empty() ガードでドロップ。
        expect_no_event(b"");
    }

    #[test]
    fn unknown_hook_event_produces_no_event() {
        // JSON は妥当だが hook_event_name が未知 → AgentStatus::from_hook_event
        // が None → 無イベント。
        expect_no_event(br#"{"hook_event_name":"TotallyUnknown","session_id":"s3"}"#);
    }
}
