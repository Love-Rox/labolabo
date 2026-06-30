import Foundation
import CoreServices

/// Recursively watches a directory via FSEvents and invokes `onChange` (on a
/// background queue) when anything under it changes. FSEvents' own `latency`
/// coalesces bursts, so an agent editing many files yields few callbacks.
///
/// Not `Sendable`; create/start/stop from one place. The callback hops wherever
/// the caller dispatches it (typically a `Task` onto the main actor).
public final class FileWatcher {
    private let path: String
    private let latency: TimeInterval
    private let onChange: () -> Void
    private var stream: FSEventStreamRef?
    private let queue = DispatchQueue(label: "com.love-rox.labolabo.filewatcher")

    public init(path: URL, latency: TimeInterval = 0.4, onChange: @escaping () -> Void) {
        self.path = path.path
        self.latency = latency
        self.onChange = onChange
    }

    public func start() {
        guard stream == nil else { return }

        var context = FSEventStreamContext(
            version: 0,
            info: Unmanaged.passUnretained(self).toOpaque(),
            retain: nil,
            release: nil,
            copyDescription: nil
        )

        let callback: FSEventStreamCallback = { _, info, _, _, _, _ in
            guard let info else { return }
            let watcher = Unmanaged<FileWatcher>.fromOpaque(info).takeUnretainedValue()
            watcher.onChange()
        }

        let flags = FSEventStreamCreateFlags(
            kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer
        )

        guard let stream = FSEventStreamCreate(
            kCFAllocatorDefault,
            callback,
            &context,
            [path] as CFArray,
            FSEventStreamEventId(kFSEventStreamEventIdSinceNow),
            latency,
            flags
        ) else { return }

        FSEventStreamSetDispatchQueue(stream, queue)
        FSEventStreamStart(stream)
        self.stream = stream
    }

    public func stop() {
        guard let stream else { return }
        FSEventStreamStop(stream)
        FSEventStreamInvalidate(stream)
        FSEventStreamRelease(stream)
        self.stream = nil
    }

    deinit {
        stop()
    }
}
