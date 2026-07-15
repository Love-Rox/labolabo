//! The control protocol's AF_UNIX transport (`docs/control-protocol.md`
//! §3): unlike `crate::hooks`'s one-way "fire event, forget" bus, this is
//! bidirectional request/response -- **one connection = one request, one
//! response**: the client writes its request then half-closes the write
//! side (`shutdown(SHUT_WR)`), the server reads to EOF, calls its handler,
//! writes the response, and closes; the client then reads the response to
//! EOF. No length prefix, no newline framing -- EOF marks each side's
//! message boundary, the same choice `crate::hooks`'s socket made
//! (docs/hooks-protocol.md §4), just used in both directions here instead
//! of one.
//!
//! [`ControlServer`] (the accept-loop/socket-bind half, generic over a
//! caller-supplied synchronous `Vec<u8> -> Vec<u8>` handler) and
//! [`send_control_request`] (the client half, a free function mirroring
//! `crate::hooks::forward_hook`'s connect/write/close shape, extended with
//! reading a response back before closing) are the two pieces;
//! `labolabo-app::control` wires `ControlServer`'s handler to a channel
//! that hands each request to the gpui main thread and blocks for its
//! reply (see that module's doc comment) -- this crate has no gpui
//! dependency, so the actual Task/tab-mutation dispatch necessarily lives
//! there, not here.
//!
//! A Windows transport (Named Pipe, per docs/control-protocol.md §9) is
//! future work and has no stub here yet -- only this comment, same as
//! `crate::hooks`'s equivalent note.

/// A synchronous request handler: raw request bytes in, raw response bytes
/// out. Runs on the accept-loop thread itself (see [`ControlServer::start`]'s
/// doc comment, `#[cfg(unix)]` module below) -- a handler that blocks (e.g.
/// `labolabo-app`'s: waiting for the gpui main thread's reply) simply makes
/// that one connection's round-trip take longer; the next connection isn't
/// accepted until this one's handler returns -- same "no request
/// concurrency" simplicity `crate::hooks`'s `run_server`/`handle_client`
/// already has for the one-way case (docs/control-protocol.md §3).
pub type ControlHandler = Box<dyn Fn(Vec<u8>) -> Vec<u8> + Send + Sync + 'static>;

#[cfg(unix)]
mod unix_transport {
    use super::ControlHandler;
    use std::fs;
    use std::io::{Read, Write};
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// Per-app-instance AF_UNIX (`SOCK_STREAM`) control socket server.
    /// Structurally a twin of `crate::hooks::UnixSocketEventTransport` --
    /// same bind/chmod/accept-loop/`stop()` shape (docs/control-protocol.md
    /// §3 mirroring docs/hooks-protocol.md §4/§8) -- with one behavioral
    /// difference: [`handle_client`] writes a response back before closing,
    /// since this channel is a request/response RPC, not a fire-and-forget
    /// event bus.
    pub struct ControlServer {
        socket_path: String,
        inner: Arc<Inner>,
    }

    struct Inner {
        socket_path: PathBuf,
        handler: Mutex<Option<ControlHandler>>,
        running: AtomicBool,
        started_once: AtomicBool,
        /// Raw fd of the bound listener while the accept loop is live, or
        /// -1 otherwise -- see `crate::hooks::UnixSocketEventTransport`'s
        /// identical field for why `stop()` needs this (unblocking a
        /// blocked `accept()` from another thread).
        listen_fd: AtomicI32,
    }

    impl ControlServer {
        pub fn new(socket_path: impl Into<String>) -> Self {
            let socket_path = socket_path.into();
            Self {
                socket_path: socket_path.clone(),
                inner: Arc::new(Inner {
                    socket_path: PathBuf::from(socket_path),
                    handler: Mutex::new(None),
                    running: AtomicBool::new(false),
                    started_once: AtomicBool::new(false),
                    listen_fd: AtomicI32::new(-1),
                }),
            }
        }

        pub fn socket_path(&self) -> &str {
            &self.socket_path
        }

        /// Registers the handler. Must be called before [`Self::start`] --
        /// a request that arrives before a handler is registered gets the
        /// "no handler registered" fallback response (see
        /// [`handle_client`]), it does not block waiting for one.
        pub fn set_handler(&mut self, handler: ControlHandler) {
            *self.inner.handler.lock().unwrap() = Some(handler);
        }

        /// Binds and starts the accept loop on a dedicated thread (blocking
        /// `accept()`/`read()`/`write()`, same reasoning as `crate::hooks`'s
        /// bus: infrequent, one-request-at-a-time traffic doesn't need a
        /// worker pool). At most once per instance -- mirrors
        /// `crate::hooks::UnixSocketEventTransport::start`'s
        /// `started_once` guard (no restart after `stop()`).
        pub fn start(&mut self) {
            if self.inner.started_once.swap(true, Ordering::SeqCst) {
                return;
            }
            let inner = Arc::clone(&self.inner);
            let _ = thread::Builder::new()
                .name("labolabo.control.server".to_string())
                .spawn(move || run_server(&inner));
        }

        pub fn stop(&mut self) {
            self.inner.running.store(false, Ordering::SeqCst);
            let fd = self.inner.listen_fd.swap(-1, Ordering::SeqCst);
            if fd >= 0 {
                // SAFETY: `fd` was obtained from `UnixListener::as_raw_fd()`
                // on the still-live listener owned by the accept-loop
                // thread; `shutdown(2)` on a valid fd is safe and merely
                // unblocks that thread's blocked `accept()` call -- see
                // `crate::hooks::UnixSocketEventTransport::stop`'s identical
                // reasoning.
                unsafe {
                    libc::shutdown(fd, libc::SHUT_RDWR);
                }
            }
            let _ = fs::remove_file(&self.inner.socket_path);
        }
    }

    fn run_server(inner: &Arc<Inner>) {
        // Clean up a stale socket file from a previous run before binding
        // (docs/control-protocol.md §3, same "unlink then bind" convention
        // as docs/hooks-protocol.md §4).
        let _ = fs::remove_file(&inner.socket_path);

        // Best-effort: ensure the parent directory exists and is
        // owner-only. Only newly-created directories in the chain get this
        // mode -- matches `crate::hooks::run_server`'s identical comment.
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

        // Only the owning user may connect (docs/control-protocol.md §3/§9).
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

    /// One request/response round trip (docs/control-protocol.md §3): read
    /// the request to EOF (the client half-closes its write side once it's
    /// sent the whole request), call the handler, write the response, then
    /// let `stream` close on drop -- the client's own `read_to_end` sees
    /// EOF right after these bytes.
    fn handle_client(inner: &Inner, mut stream: UnixStream) {
        let mut data = Vec::new();
        // Ignore read errors, same "keep whatever arrived" reasoning as
        // `crate::hooks::handle_client`.
        let _ = stream.read_to_end(&mut data);

        let response = match inner.handler.lock().unwrap().as_ref() {
            Some(handler) => handler(data),
            None => {
                br#"{"ok":false,"error":"labolabo-app: control server has no handler registered"}"#
                    .to_vec()
            }
        };
        let _ = stream.write_all(&response);
    }
}

#[cfg(unix)]
pub use unix_transport::ControlServer;

/// The CLI/agent side of one control request (docs/control-protocol.md §3):
/// connect, write the whole request, half-close the write side (so the
/// server's `read_to_end` sees EOF without needing a length prefix), then
/// read the whole response before the connection closes. Mirrors
/// `crate::hooks::forward_hook`'s connect/write shape, extended with the
/// read-the-reply half a fire-and-forget hook event doesn't need.
#[cfg(unix)]
pub fn send_control_request(socket_path: &str, request_bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path)?;
    stream.write_all(request_bytes)?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    Ok(response)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::control_protocol::{parse_request, ControlCommand, ControlResponse};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::Duration;

    fn temp_socket_path(label: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let short = format!("{:x}{:x}", nanos as u32, n);
        std::env::temp_dir().join(format!("lb-ctl-{label}-{short}.sock"))
    }

    /// Retries `send_control_request` for a moment -- the accept-loop
    /// thread needs time to bind after `start()`, same reasoning as
    /// `crate::hooks`'s own socket tests' `send_payload` retry loop.
    fn send_with_retry(path: &std::path::Path, request_bytes: &[u8]) -> std::io::Result<Vec<u8>> {
        let mut last_err = None;
        for _ in 0..150 {
            match send_control_request(path.to_str().unwrap(), request_bytes) {
                Ok(response) => return Ok(response),
                Err(err) => {
                    last_err = Some(err);
                    thread::sleep(Duration::from_millis(20));
                }
            }
        }
        Err(last_err.unwrap())
    }

    #[test]
    fn round_trip_echoes_through_a_stub_handler() {
        let path = temp_socket_path("echo");
        let mut server = ControlServer::new(path.to_str().unwrap());
        server.set_handler(Box::new(|bytes| {
            let mut out = b"echo:".to_vec();
            out.extend(bytes);
            out
        }));
        server.start();

        let response = send_with_retry(&path, b"hello").expect("round trip should succeed");
        assert_eq!(response, b"echo:hello");

        server.stop();
    }

    #[test]
    fn no_handler_registered_yields_the_default_error_response() {
        let path = temp_socket_path("no-handler");
        let mut server = ControlServer::new(path.to_str().unwrap());
        server.start();

        let response = send_with_retry(&path, b"{}").expect("connection should succeed");
        let value: serde_json::Value = serde_json::from_slice(&response).unwrap();
        assert_eq!(value["ok"], false);

        server.stop();
    }

    #[test]
    fn sequential_requests_each_get_their_own_response() {
        let path = temp_socket_path("sequential");
        let mut server = ControlServer::new(path.to_str().unwrap());
        server.set_handler(Box::new(|bytes| {
            let n: u32 = std::str::from_utf8(&bytes).unwrap().parse().unwrap();
            (n * 2).to_string().into_bytes()
        }));
        server.start();

        for i in 0..5u32 {
            let response = send_with_retry(&path, i.to_string().as_bytes()).unwrap();
            assert_eq!(std::str::from_utf8(&response).unwrap(), (i * 2).to_string());
        }

        server.stop();
    }

    /// End-to-end through the *real* protocol types (not just raw bytes):
    /// a handler that actually parses the request with
    /// `control_protocol::parse_request`, dispatches it with
    /// `ControlCommand::from_request`, and replies with a real
    /// `ControlResponse` -- proving the whole pipeline (this module's
    /// transport + `control_protocol`'s pure logic) works together over a
    /// real socket, without any gpui/app-layer involvement.
    #[test]
    fn full_protocol_round_trip_task_list_through_a_pure_handler() {
        let path = temp_socket_path("protocol");
        let mut server = ControlServer::new(path.to_str().unwrap());
        server.set_handler(Box::new(|bytes| {
            let request = match parse_request(&bytes) {
                Ok(request) => request,
                Err(err) => return ControlResponse::err(err).to_bytes(),
            };
            let command = match ControlCommand::from_request(&request) {
                Ok(command) => command,
                Err(err) => return ControlResponse::err(err).to_bytes(),
            };
            match command {
                ControlCommand::TaskList => {
                    ControlResponse::ok(serde_json::json!({"tasks": []})).to_bytes()
                }
                _ => ControlResponse::err("unexpected command in this test").to_bytes(),
            }
        }));
        server.start();

        let request =
            crate::control_protocol::ControlRequest::new("task_list", serde_json::json!({}));
        let response_bytes = send_with_retry(&path, &request.to_bytes()).unwrap();
        let response = crate::control_protocol::parse_response(&response_bytes).unwrap();
        assert!(response.ok);
        assert_eq!(response.result, Some(serde_json::json!({"tasks": []})));

        server.stop();
    }

    #[test]
    fn unknown_command_over_the_wire_gets_an_error_response_not_a_dropped_connection() {
        let path = temp_socket_path("unknown-command");
        let mut server = ControlServer::new(path.to_str().unwrap());
        server.set_handler(Box::new(|bytes| {
            let request = match parse_request(&bytes) {
                Ok(request) => request,
                Err(err) => return ControlResponse::err(err).to_bytes(),
            };
            match ControlCommand::from_request(&request) {
                Ok(_) => ControlResponse::ok_empty().to_bytes(),
                Err(err) => ControlResponse::err(err).to_bytes(),
            }
        }));
        server.start();

        let request = crate::control_protocol::ControlRequest::new(
            "nonexistent_command",
            serde_json::json!({}),
        );
        let response_bytes = send_with_retry(&path, &request.to_bytes()).unwrap();
        let response = crate::control_protocol::parse_response(&response_bytes).unwrap();
        assert!(!response.ok);
        assert!(response.error.unwrap().contains("nonexistent_command"));

        server.stop();
    }
}
