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

    @discardableResult
    public static func run(_ arguments: [String], in directory: URL) async throws -> String {
        await gate.acquire()
        let result: Result<ProcessRunner.Output, Error>
        do {
            result = .success(try await ProcessRunner.run(
                executable: URL(fileURLWithPath: "/usr/bin/env"),
                arguments: ["git"] + arguments,
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
}
