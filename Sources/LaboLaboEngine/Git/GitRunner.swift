import Foundation

/// Thrown when a `git` invocation exits non-zero.
public struct GitCommandError: Error, CustomStringConvertible {
    public let arguments: [String]
    public let exitCode: Int32
    public let stderr: String

    public var description: String {
        "git \(arguments.joined(separator: " ")) failed (exit \(exitCode)): \(stderr.trimmingCharacters(in: .whitespacesAndNewlines))"
    }
}

/// Runs the system `git` binary and returns its stdout.
///
/// 実行はスレッドを占有しない `ProcessRunner.run` に委譲する。ここで GCD の
/// ワーカーをブロックすると、並行 git が重なったときにプール（64 本）が待ち側で
/// 埋まり、アプリ全体（終了時のウィンドウ状態保存を含む）が固まる。
/// 同時実行数もゲートで抑え、org ディレクトリを開いた直後の一斉 refresh で
/// git プロセスが暴発しないようにする。
public enum GitRunner {

    private static let gate = ConcurrencyGate(limit: 16)

    /// `git` の絶対パス解決を差し替え可能にする（既定は本物の `ToolLocator`）。
    /// テストで偽のロケータを注入するためのフックで、通常呼び出しでは指定不要。
    @discardableResult
    public static func run(
        _ arguments: [String],
        in directory: URL,
        locator: ToolLocating.Type = ToolLocator.self
    ) async throws -> String {
        await gate.acquire()
        let (executable, spawnArguments) = resolveGit(locator: locator, arguments: arguments)
        let result: Result<ProcessRunner.Output, Error>
        do {
            result = .success(try await ProcessRunner.run(
                executable: executable,
                arguments: spawnArguments,
                in: directory
            ))
        } catch {
            result = .failure(error)
        }
        await gate.release()

        let output = try result.get()
        guard output.status == 0 else {
            throw GitCommandError(
                arguments: arguments,
                exitCode: output.status,
                stderr: output.stderr
            )
        }
        return output.stdout
    }

    /// git の起動コマンドを組み立てる。
    ///
    /// `ToolLocating` で解決できればその絶対パスを直接 `posix_spawn` する
    /// （`env` を経由しない）。解決できない（PATH に無い等）場合は、これまでの
    /// 挙動を変えないよう `/usr/bin/env git` にフォールバックし、`env` 自身の
    /// PATH 探索に委ねる。
    private static func resolveGit(
        locator: ToolLocating.Type,
        arguments: [String]
    ) -> (executable: URL, arguments: [String]) {
        if let git = locator.locate("git") {
            return (git, arguments)
        }
        return (URL(fileURLWithPath: "/usr/bin/env"), ["git"] + arguments)
    }
}
