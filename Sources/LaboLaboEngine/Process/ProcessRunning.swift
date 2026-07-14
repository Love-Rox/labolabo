import Foundation

/// プロセス起動の抽象化。
///
/// macOS 版は posix_spawn/kqueue ベースの `ProcessRunner` が実装する。将来
/// Windows 版を足すときは CreateProcess ベースの実装をこの protocol に差し込める
/// （呼び出し側は `ProcessRunning` にしか依存しない形にしていく）。
///
/// シグネチャは既存の `ProcessRunner.run` と同一（挙動を変えず契約として明示するのが目的）。
public protocol ProcessRunning {
    static func run(
        executable: URL,
        arguments: [String],
        in directory: URL?,
        environment: [String: String]?
    ) async throws -> ProcessRunner.Output
}

extension ProcessRunner: ProcessRunning {}
