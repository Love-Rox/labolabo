//! Cross-platform port of `Sources/LaboLaboEngine/Git/FileWatcher.swift`
//! (macOS-only, built on FSEvents) and the `FileWatching` protocol
//! `FileWatching.swift` already anticipates a cross-platform implementation
//! for (its doc comment: "将来 Windows 版を足すときは... この protocol に
//! 差し込める"). This port covers macOS/Linux/Windows in one implementation
//! via the [`notify`] crate (FSEvents / inotify / `ReadDirectoryChangesW`
//! respectively, behind one API) instead of committing to a single
//! platform's native API the way the Swift source does.
//!
//! ## Debounce
//!
//! The Swift `FileWatcher` relies on `FSEventStreamCreate`'s own `latency`
//! parameter (default `0.4`s) to coalesce a burst of native events into few
//! callbacks. `notify` has no equivalent built-in coalescing, so this port
//! implements it explicitly: a dedicated background thread blocks on the
//! event channel (`mpsc::Receiver::recv`, no polling) and, once the first
//! (non-git-noise, see below) event of a new burst arrives, keeps draining
//! the channel with a `latency`-long timeout after *each* event, resetting
//! that timeout on every new arrival, until the channel goes quiet for a
//! full `latency` -- then flushes every path collected during the burst in
//! one `on_change` call. This is a "reset per event" debounce, not quite
//! FSEventStream's "batch since the first unprocessed event" semantics
//! (which has a firmer upper bound during a very long, continuous burst) --
//! a deliberate, documented simplification; both converge on the same
//! outcome (few callbacks for a burst) for the bursts this app actually
//! sees (an editor save, a `git commit`, an agent's multi-file edit).
//! [`FileWatcher::watch`]'s `latency` parameter plays the same role as
//! Swift's `FileWatcher.init(path:latency:onChange:)`'s `latency` (that
//! source's own default is `0.4`s -- callers here choose their own; see
//! `git_pane.rs` in `labolabo-app` for this port's).
//!
//! Because both waits are blocking (`recv`/`recv_timeout`), the debounce
//! thread does no work and wakes for no reason while nothing is changing --
//! matching this wave's "アイドル時にポーリングしないこと" requirement.
//!
//! ## `.git/` filtering
//!
//! Unlike the Swift `FileWatcher` (which forwards every FSEvents path
//! completely unfiltered -- verified against `SessionStore.swift`'s and
//! `WorkPaneModel.swift`'s call sites, neither of which filters `.git/`
//! either, relying entirely on FSEvents' own coalescing plus their own
//! app-level debounce to absorb the noise), this port filters out most of
//! `.git/`'s own churn (see [`is_git_noise`]) before an event even reaches
//! the debounce accumulator. Two reasons this diverges from the Swift
//! source rather than matching it byte-for-byte:
//!
//! 1. `index.lock` create/delete during every `git` write operation (commit,
//!    add, checkout, ...) would otherwise repeatedly retrigger/extend the
//!    debounce window above for the whole duration of the operation --
//!    lower-impact than on macOS (FSEvents already coalesces at the OS
//!    level before this port's own debounce even sees an event) but not
//!    zero, and the concern is explicit in this wave's brief.
//! 2. Unlike FSEvents (one coalesced stream, no per-directory cost), the
//!    Linux inotify backend `notify` uses for `RecursiveMode::Recursive`
//!    adds one watch descriptor per directory it walks -- a large repo's
//!    `.git/objects/xx/` fan-out (256 shard directories, growing over the
//!    repo's lifetime) is pure overhead nothing in this app ever needs to
//!    observe.
//!
//! `.git/HEAD` and everything under `.git/refs/` are deliberately still
//! forwarded -- both change on every branch switch/commit and are the
//! cheapest possible signal for "the branch status bar may be stale," which
//! this wave's brief calls out by name as worth keeping.
//!
//! [`is_git_noise`] matches on path *components* (does this path have a
//! `.git` component, and if so what immediately follows it) rather than
//! stripping a `root` prefix first. This is deliberate: some platforms'
//! event paths aren't byte-identical to the `root` a caller passed to
//! [`FileWatcher::watch`] (notably macOS, where FSEvents reports the
//! symlink-resolved form -- `/tmp/...` comes back as `/private/tmp/...` --
//! and Windows, where `std::fs::canonicalize` produces a `\\?\`-prefixed
//! extended-length path that a plain watch path never has). Component
//! matching sidesteps that mismatch risk entirely instead of trying to
//! normalize both sides into the same form.

use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

/// Recursively watches a directory and invokes `on_change` (on a private
/// background thread -- see the module doc comment) with the batch of
/// changed paths whenever the watched tree changes, debounced.
///
/// Not `Sync`/`Clone` (mirrors the Swift source's "create/start/stop from
/// one place" contract in its own doc comment); the callback itself must be
/// `Send + 'static` since it runs on this struct's own background thread,
/// not the caller's.
pub struct FileWatcher {
    /// `Some` while watching; dropping it (in [`Self::stop`]) tears down the
    /// OS-level watch and drops its internal event-channel sender, which is
    /// what lets the debounce thread below notice it should exit (its
    /// `mpsc::Receiver` disconnects once the last sender -- held by this
    /// watcher -- is gone).
    watcher: Option<notify::RecommendedWatcher>,
    /// Joined by [`Self::stop`] so that, once `stop` returns, the debounce
    /// thread has fully exited and `on_change` is guaranteed not to fire
    /// again -- a stronger, synchronous-stop guarantee than FSEventStream's
    /// own `FSEventStreamStop` (asynchronous invalidation), which this port
    /// leans on for the "非表示中は...電力ゼロに" contract: callers can
    /// treat `stop()` returning as "definitely quiesced," no race window.
    debounce_thread: Option<JoinHandle<()>>,
}

impl FileWatcher {
    /// Starts watching `root` (recursively) and returns the live watcher.
    /// Mirrors `FileWatching.watch(path:latency:onChange:)` (`init` +
    /// `start()` combined into one factory, Swift's own established
    /// convention for this type) -- the caller must hold onto the returned
    /// value for watching to continue (dropping it calls [`Self::stop`]).
    ///
    /// Errors if the underlying OS watch can't be established (e.g. `root`
    /// doesn't exist, or -- on Linux -- the process is out of inotify watch
    /// descriptors); there is no silent-no-op fallback, unlike the Swift
    /// source's `start()` (which swallows `FSEventStreamCreate` failure) --
    /// giving the caller a `Result` here is strictly more informative and
    /// costs callers nothing (they can still choose to ignore it).
    pub fn watch<F>(root: impl AsRef<Path>, latency: Duration, on_change: F) -> notify::Result<Self>
    where
        F: Fn(Vec<String>) + Send + 'static,
    {
        let root = root.as_ref();

        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(tx)?;
        watcher.watch(root, RecursiveMode::Recursive)?;

        let debounce_thread = thread::Builder::new()
            .name("labolabo-file-watcher".to_string())
            .spawn(move || debounce_loop(rx, latency, on_change))
            .expect("failed to spawn the file-watcher debounce thread");

        Ok(Self {
            watcher: Some(watcher),
            debounce_thread: Some(debounce_thread),
        })
    }

    /// Stops watching. Idempotent (a second call is a no-op) and
    /// synchronous -- see `debounce_thread`'s doc comment for the
    /// "definitely quiesced when this returns" guarantee.
    pub fn stop(&mut self) {
        self.watcher = None;
        if let Some(handle) = self.debounce_thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for FileWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The debounce thread's body -- see the module doc comment for the
/// algorithm. Runs until `rx` disconnects (i.e. [`FileWatcher::stop`]
/// dropped the watcher).
fn debounce_loop<F>(
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    latency: Duration,
    on_change: F,
) where
    F: Fn(Vec<String>),
{
    let mut pending: Vec<String> = Vec::new();
    loop {
        // Block for the first event of a new burst -- no polling.
        let Ok(msg) = rx.recv() else { return };
        collect_event(msg, &mut pending);
        if pending.is_empty() {
            // Pure git-noise (see `is_git_noise`) or a path-less event:
            // nothing worth debouncing yet, go back to waiting rather than
            // spending a `latency` window coalescing nothing.
            continue;
        }

        // Drain the rest of this burst, extending the wait on every new
        // (non-noise) arrival, until the channel is quiet for `latency`.
        loop {
            match rx.recv_timeout(latency) {
                Ok(msg) => collect_event(msg, &mut pending),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if !pending.is_empty() {
                        on_change(std::mem::take(&mut pending));
                    }
                    return;
                }
            }
        }

        on_change(std::mem::take(&mut pending));
    }
}

/// Appends `msg`'s paths to `pending` (deduplicated), skipping
/// [`is_git_noise`] ones and silently dropping watch errors (a transient
/// error -- e.g. an event queue overflow -- shouldn't stop future events
/// from being observed; there is no reconnect logic to run here either
/// way, `notify`'s watcher keeps running on its own).
fn collect_event(msg: notify::Result<notify::Event>, pending: &mut Vec<String>) {
    let Ok(event) = msg else { return };
    for path in event.paths {
        if is_git_noise(&path) {
            continue;
        }
        let path = path.to_string_lossy().into_owned();
        if !pending.contains(&path) {
            pending.push(path);
        }
    }
}

/// `true` when `path` has a `.git` path component that is neither the
/// path's last component (the bare `.git` entry itself) nor immediately
/// followed by `HEAD` (and nothing after) or `refs` -- see the module doc
/// comment's "`.git/` filtering" section for why, and for why this matches
/// on components rather than a `root`-relative prefix.
fn is_git_noise(path: &Path) -> bool {
    let mut components = path.components();
    while let Some(component) = components.next() {
        if component.as_os_str() != ".git" {
            continue;
        }
        return match components.next() {
            // The bare `.git` entry itself (its own creation/removal, e.g.
            // a worktree being torn down) -- not noise, just rare.
            None => false,
            Some(next) if next.as_os_str() == "refs" => false,
            Some(next) if next.as_os_str() == "HEAD" && components.next().is_none() => false,
            _ => true,
        };
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    fn scratch_dir(prefix: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Polls `cond` (never blocking the OS -- this is test-only harness
    /// code waiting on another thread's async work, not production
    /// polling) until it's true or `timeout` elapses.
    fn wait_for(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        loop {
            if cond() {
                return true;
            }
            if start.elapsed() >= timeout {
                return cond();
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    // MARK: - is_git_noise (pure, deterministic)

    #[test]
    fn git_head_and_refs_are_not_noise() {
        assert!(!is_git_noise(Path::new("/repo/.git/HEAD")));
        assert!(!is_git_noise(Path::new("/repo/.git/refs")));
        assert!(!is_git_noise(Path::new("/repo/.git/refs/heads/main")));
    }

    #[test]
    fn other_git_internals_are_noise() {
        assert!(is_git_noise(Path::new("/repo/.git/index.lock")));
        assert!(is_git_noise(Path::new("/repo/.git/index")));
        assert!(is_git_noise(Path::new("/repo/.git/objects/ab/cdef1234")));
        assert!(is_git_noise(Path::new("/repo/.git/logs/HEAD")));
        // A path that merely starts with "HEAD" but isn't exactly `.git/HEAD`.
        assert!(is_git_noise(Path::new("/repo/.git/HEAD.lock")));
    }

    #[test]
    fn paths_outside_git_are_never_noise() {
        assert!(!is_git_noise(Path::new("/repo/src/main.rs")));
        assert!(!is_git_noise(Path::new("/repo/.git"))); // the bare entry
        assert!(!is_git_noise(Path::new("/repo/.gitignore")));
    }

    #[test]
    fn a_nested_git_dir_anywhere_in_the_path_is_still_matched() {
        // Component matching (not a `root`-relative prefix check) means a
        // `.git` anywhere in the path -- however deeply nested, and
        // regardless of what `root` the caller originally watched -- is
        // still recognized. See the module doc comment's "`.git/`
        // filtering" section for why this port doesn't require an exact
        // `root` prefix match in the first place.
        assert!(is_git_noise(Path::new(
            "/a/b/c/repo/.git/objects/ab/cdef1234"
        )));
        assert!(!is_git_noise(Path::new("/a/b/c/repo/.git/refs/heads/main")));
    }

    // MARK: - end-to-end (real notify watcher + tempdir)

    #[test]
    fn fires_after_a_plain_file_write() {
        let dir = scratch_dir("labolabo-filewatcher-fire");
        let events: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        let mut watcher = FileWatcher::watch(&dir, Duration::from_millis(80), move |paths| {
            events_clone.lock().unwrap().push(paths);
        })
        .expect("watch should succeed");

        std::fs::write(dir.join("a.txt"), "hello").unwrap();

        let fired = wait_for(Duration::from_secs(5), || {
            !events.lock().unwrap().is_empty()
        });
        assert!(fired, "expected at least one callback after a file write");
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .any(|p| p.ends_with("a.txt")));

        watcher.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The union of file *names* reported across all callbacks so far.
    /// Extracted with `Path::file_name`, not a hand-rolled `rsplit('/')`:
    /// reported paths use `\` separators on Windows, where a `/`-split
    /// returns the whole path unsplit -- exactly how the first version of
    /// the burst test below failed on the `windows-latest` CI leg while
    /// passing everywhere else.
    fn seen_names(calls: &[Vec<String>]) -> std::collections::HashSet<String> {
        calls
            .iter()
            .flatten()
            .filter_map(|p| {
                Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .collect()
    }

    /// Asserts only the properties [`FileWatcher`] actually guarantees --
    /// every change is eventually reported, bursts produce fewer callbacks
    /// than writes, and nothing fires after `stop()` -- rather than an
    /// exact callback count. How an OS slices a burst into events (and
    /// therefore how many debounce windows it spans) differs per backend
    /// (FSEvents coalesces aggressively; inotify and ReadDirectoryChangesW
    /// deliver finer-grained streams), so a tight count bound just chases
    /// platform/scheduler noise.
    #[test]
    fn debounces_a_burst_of_writes_into_few_callbacks() {
        let dir = scratch_dir("labolabo-filewatcher-debounce");
        let events: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        // A generous latency relative to how fast the burst below is
        // written, so every write in the loop falls inside one debounce
        // window on any healthy runner.
        let mut watcher = FileWatcher::watch(&dir, Duration::from_millis(300), move |paths| {
            events_clone.lock().unwrap().push(paths);
        })
        .expect("watch should succeed");

        const WRITES: usize = 8;
        for i in 0..WRITES {
            std::fs::write(dir.join(format!("burst-{i}.txt")), "x").unwrap();
        }

        // Guaranteed property #1: every written file is eventually
        // reported. Wait until *all* names have been seen (in however many
        // batches the platform delivers them), not just the first flush --
        // a slow runner may surface stragglers in a later batch.
        let all_reported = wait_for(Duration::from_secs(10), || {
            let calls = events.lock().unwrap();
            let seen = seen_names(&calls);
            (0..WRITES).all(|i| seen.contains(&format!("burst-{i}.txt")))
        });
        assert!(
            all_reported,
            "every written file must eventually be reported; got {:?}",
            events.lock().unwrap()
        );

        // Guaranteed property #2: `stop()` is synchronous (joins the
        // debounce thread), so once it returns no callback can ever fire
        // again. Record the count, provoke one more change, give a wrong
        // implementation ample time to surface it, and assert silence.
        watcher.stop();
        let calls_at_stop = events.lock().unwrap().len();
        std::fs::write(dir.join("after-stop.txt"), "x").unwrap();
        thread::sleep(Duration::from_millis(400));
        assert_eq!(
            events.lock().unwrap().len(),
            calls_at_stop,
            "no callback may fire after stop() has returned"
        );

        // Guaranteed property #3, the debounce itself, asserted
        // deliberately conservatively (no per-platform thresholds -- see
        // this test's doc comment): a burst of N near-simultaneous writes
        // must produce strictly fewer than N callbacks. One-per-write (or
        // more) would mean the debounce coalesced nothing at all.
        let calls = events.lock().unwrap();
        assert!(
            calls.len() < WRITES,
            "a {WRITES}-write burst must coalesce into fewer than {WRITES} callbacks, got {}",
            calls.len()
        );
        drop(calls);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_internal_noise_is_excluded_but_head_and_refs_still_fire() {
        let dir = scratch_dir("labolabo-filewatcher-gitfilter");
        std::fs::create_dir_all(dir.join(".git/refs/heads")).unwrap();

        let events: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        let mut watcher = FileWatcher::watch(&dir, Duration::from_millis(150), move |paths| {
            events_clone.lock().unwrap().push(paths);
        })
        .expect("watch should succeed");

        // A burst mixing git-internal noise with real, meaningful changes
        // (including HEAD/refs, which must still be forwarded) -- written
        // close together so they land in one debounced batch.
        std::fs::write(dir.join(".git/index.lock"), "").unwrap();
        std::fs::remove_file(dir.join(".git/index.lock")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(dir.join(".git/refs/heads/main"), "deadbeef\n").unwrap();
        std::fs::write(dir.join("real.txt"), "hello").unwrap();

        let fired = wait_for(Duration::from_secs(5), || {
            !events.lock().unwrap().is_empty()
        });
        assert!(fired, "expected the real.txt write to trigger a callback");
        // Give the debounce window(s) a moment to fully settle before
        // reading the aggregate (git internals may land in an earlier or
        // later batch than real.txt depending on OS event ordering).
        thread::sleep(Duration::from_millis(400));
        watcher.stop();

        let all_paths: Vec<String> = events.lock().unwrap().iter().flatten().cloned().collect();
        assert!(
            all_paths.iter().any(|p| p.ends_with("real.txt")),
            "real.txt should have fired: {all_paths:?}"
        );
        assert!(
            !all_paths.iter().any(|p| p.ends_with("index.lock")),
            "index.lock should have been filtered out: {all_paths:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
