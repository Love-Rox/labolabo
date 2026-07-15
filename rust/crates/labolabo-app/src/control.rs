//! `labolabo-app`'s control-protocol wiring (`docs/control-protocol.md`,
//! `plans/012-task-model-and-control-cli.md` §2): the app-layer half of
//! `labolabo_core::control`'s generic AF_UNIX request/response server --
//! same "resident accept loop on its own thread, bridged into gpui via a
//! channel" shape as `crate::hooks::HookRuntime`, but bidirectional: each
//! request needs a reply, so the bridge hands back a
//! `std::sync::mpsc::Sender<Vec<u8>>` per request instead of the hooks
//! bridge's fire-and-forget `AgentStatusEvent`.
//!
//! [`ControlRuntime::new`] starts the server with a handler that hands each
//! request off over an unbounded channel and blocks (with a timeout) for
//! the reply -- this blocking happens on `labolabo_core::control::
//! ControlServer`'s accept-loop thread, *not* the gpui main thread, so a
//! slow/hung gpui update can't wedge the socket accept loop itself (it only
//! delays that one connection's own reply). [`spawn_control_bridge`] then
//! does the actual command dispatch via `LaboLaboApp::dispatch_control`,
//! routed through a `gpui::WindowHandle` (not the plain `WeakEntity` update
//! `hooks::spawn_agent_event_bridge` uses) because command handlers like
//! `open_tab_for_control`/`select_task` need a live `&mut Window` (e.g. to
//! move keyboard focus into a freshly opened tab), which only
//! `WindowHandle::update` provides from outside a render pass.

use std::time::Duration;

use futures::channel::mpsc;
use futures::StreamExt;
use gpui::{Context, Task as GpuiTask, WindowHandle};

use labolabo_core::control::ControlServer;
use labolabo_core::{control_socket_path_from_uuid, ControlResponse};

use crate::app::LaboLaboApp;

/// Base directory for the app's control socket -- shared with the hooks
/// socket's base dir (docs/control-protocol.md §3), distinguished by the
/// `control-` filename prefix `control_socket_path_from_uuid` applies.
const SOCKET_BASE_DIR: &str = "/tmp/labolabo";

/// How long the control server's accept-loop thread waits for the gpui main
/// thread's reply before giving up and answering the client with a timeout
/// error (docs/control-protocol.md §6) -- generous, since `tab_open` spawns
/// a real PTY, but bounded so a wedged gpui event loop doesn't hang a
/// client (e.g. a Claude Code subprocess blocked on `labolabo tab open`)
/// forever.
const REPLY_TIMEOUT: Duration = Duration::from_secs(15);

/// One received request, paired with the (synchronous, `std::sync::mpsc`)
/// channel [`ControlServer`]'s accept-thread handler is blocked on for the
/// reply.
pub struct ControlEnvelope {
    pub request_bytes: Vec<u8>,
    pub reply_tx: std::sync::mpsc::Sender<Vec<u8>>,
}

/// Owns the app-wide control socket/server for the process's lifetime --
/// see this module's doc comment.
pub struct ControlRuntime {
    /// Kept alive for the process's lifetime purely so its accept-loop
    /// thread and bound socket stay live -- same reasoning as
    /// `crate::hooks::HookRuntime`'s `_bus` field.
    _server: ControlServer,
    /// This process's control socket path -- injected into every spawned
    /// pane's env as `LABOLABO_CONTROL_SOCKET` (docs/control-protocol.md
    /// §4.1), the same place `LABOLABO_PANE`/`LABOLABO_TASK` are injected.
    pub socket_path: String,
}

impl ControlRuntime {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<ControlEnvelope>) {
        let socket_uuid = uuid::Uuid::new_v4().to_string();
        Self::new_at(control_socket_path_from_uuid(&socket_uuid, SOCKET_BASE_DIR))
    }

    /// [`ControlRuntime::new`]'s whole construction, parameterized on the
    /// socket path -- private, split out for the same reason
    /// `HookRuntime::new_at` is: lets a test run the real server/channel
    /// machinery against a socket under the OS temp dir instead of the real
    /// `/tmp/labolabo`.
    fn new_at(socket_path: String) -> (Self, mpsc::UnboundedReceiver<ControlEnvelope>) {
        let (tx, rx) = mpsc::unbounded();
        let mut server = ControlServer::new(socket_path.clone());
        server.set_handler(Box::new(move |request_bytes| {
            let (reply_tx, reply_rx) = std::sync::mpsc::channel();
            if tx
                .unbounded_send(ControlEnvelope {
                    request_bytes,
                    reply_tx,
                })
                .is_err()
            {
                // The gpui-side bridge task is gone (app shutting down) --
                // answer inline rather than blocking forever on a reply
                // that will never come.
                return ControlResponse::err("labolabo-app: control bridge is not running")
                    .to_bytes();
            }
            reply_rx.recv_timeout(REPLY_TIMEOUT).unwrap_or_else(|_| {
                ControlResponse::err("labolabo-app: timed out waiting for a reply").to_bytes()
            })
        }));
        server.start();
        (
            Self {
                _server: server,
                socket_path,
            },
            rx,
        )
    }
}

/// Bridges the [`ControlEnvelope`] channel into gpui: for each request,
/// dispatches it through `window_handle` (giving `LaboLaboApp::
/// dispatch_control` a live `&mut Window`) and sends the serialized
/// response back over the request's own reply channel. If the window is
/// gone (`update` fails, e.g. mid-quit), replies with an error rather than
/// leaving the client hanging until [`REPLY_TIMEOUT`].
pub fn spawn_control_bridge(
    mut requests_rx: mpsc::UnboundedReceiver<ControlEnvelope>,
    window_handle: WindowHandle<LaboLaboApp>,
    cx: &mut Context<LaboLaboApp>,
) -> GpuiTask<()> {
    cx.spawn(async move |_this, cx| {
        while let Some(envelope) = requests_rx.next().await {
            let response_bytes = window_handle
                .update(cx, |app, window, cx| {
                    app.dispatch_control(&envelope.request_bytes, window, cx)
                })
                .unwrap_or_else(|_| {
                    ControlResponse::err("labolabo-app: window is not available").to_bytes()
                });
            let _ = envelope.reply_tx.send(response_bytes);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use labolabo_core::{parse_response, send_control_request, ControlRequest};
    use std::thread;
    use std::time::Duration as StdDuration;

    fn temp_socket_path(label: &str) -> String {
        std::env::temp_dir()
            .join(format!("lb-ctlrt-{label}-{}.sock", std::process::id()))
            .to_string_lossy()
            .into_owned()
    }

    fn send_with_retry(path: &str, request: &ControlRequest) -> Vec<u8> {
        let bytes = request.to_bytes();
        for _ in 0..150 {
            if let Ok(response) = send_control_request(path, &bytes) {
                return response;
            }
            thread::sleep(StdDuration::from_millis(20));
        }
        panic!("control request never got a response from {path}");
    }

    /// Exercises the real construction path (`ControlRuntime::new_at`: a
    /// real `ControlServer` bound to a real socket, real channel) without
    /// any gpui involvement: a request sent over the real socket arrives on
    /// the [`ControlEnvelope`] channel, and replying via `reply_tx` (as
    /// `spawn_control_bridge` would, minus the gpui `window_handle.update`
    /// hop) is what the client sees as its response. Proves the
    /// `ControlServer` handler <-> channel <-> reply wiring this module
    /// owns is correct in isolation from `LaboLaboApp::dispatch_control`
    /// (which needs a live window, exercised instead by the `tests/
    /// control_cli.rs` integration test running the real `labolabo` CLI
    /// binary against a stub handler at the `labolabo_core::control` layer).
    #[test]
    fn control_runtime_delivers_a_real_request_over_the_channel_and_replies() {
        let socket_path = temp_socket_path("runtime");
        let (runtime, mut rx) = ControlRuntime::new_at(socket_path.clone());
        assert_eq!(runtime.socket_path, socket_path);

        let handle = thread::spawn(move || {
            let request = ControlRequest::new("task_list", serde_json::json!({}));
            send_with_retry(&socket_path, &request)
        });

        let envelope =
            futures::executor::block_on(async { futures::StreamExt::next(&mut rx).await })
                .expect("the server's handler should forward the request over the channel");
        let request = labolabo_core::parse_request(&envelope.request_bytes).unwrap();
        assert_eq!(request.command, "task_list");
        envelope
            .reply_tx
            .send(ControlResponse::ok(serde_json::json!({"tasks": []})).to_bytes())
            .expect("reply channel should still be open");

        let response_bytes = handle.join().expect("client thread should not panic");
        let response = parse_response(&response_bytes).expect("valid response JSON");
        assert!(response.ok);
        assert_eq!(response.result, Some(serde_json::json!({"tasks": []})));
    }

    #[test]
    fn control_runtime_times_out_gracefully_when_nothing_answers() {
        // Not a full 15s wait -- a request whose envelope is received but
        // never replied to would hang until REPLY_TIMEOUT; instead this
        // test only checks that dropping the receiver end (simulating "the
        // bridge task is gone") makes the handler answer immediately with
        // an error rather than hanging, via the `unbounded_send` failure
        // path in `new_at`'s handler closure.
        let socket_path = temp_socket_path("no-bridge");
        let (runtime, rx) = ControlRuntime::new_at(socket_path.clone());
        drop(rx);

        let request = ControlRequest::new("task_list", serde_json::json!({}));
        let response_bytes = send_with_retry(&socket_path, &request);
        let response = parse_response(&response_bytes).unwrap();
        assert!(!response.ok);
        assert!(response
            .error
            .unwrap()
            .contains("control bridge is not running"));

        let _ = runtime;
    }
}
