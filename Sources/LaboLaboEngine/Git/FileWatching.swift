import Foundation

/// ディレクトリ配下のファイル変更監視の抽象化。
///
/// macOS 版は FSEvents ベースの `FileWatcher` が実装する。将来 Windows 版を
/// 足すときは ReadDirectoryChangesW ベースの実装をこの protocol に差し込める
/// （呼び出し側は `FileWatching` にしか依存しない形にしていく）。
///
/// `watch(path:latency:onChange:)` は `FileWatcher.init` + `start()` に相当する
/// ファクトリ。返り値のインスタンスを呼び出し側が保持し続けることで監視が続く
/// （既存の `FileWatcher` の生存管理と同じ規約）。
public protocol FileWatching: AnyObject {
    static func watch(
        path: URL,
        latency: TimeInterval,
        onChange: @escaping ([String]) -> Void
    ) -> Self

    func stop()
}

extension FileWatcher: FileWatching {
    public static func watch(
        path: URL,
        latency: TimeInterval = 0.4,
        onChange: @escaping ([String]) -> Void
    ) -> Self {
        let watcher = Self(path: path, latency: latency, onChange: onChange)
        watcher.start()
        return watcher
    }
}
