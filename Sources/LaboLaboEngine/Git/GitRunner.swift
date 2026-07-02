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
/// stdout/stderr are drained concurrently while the process runs so large diffs
/// cannot deadlock on a full pipe buffer; the call hops to a background queue so
/// `waitUntilExit()` never blocks a cooperative thread.
public enum GitRunner {

    @discardableResult
    public static func run(_ arguments: [String], in directory: URL) async throws -> String {
        try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
                process.arguments = ["git"] + arguments
                process.currentDirectoryURL = directory

                let outPipe = Pipe()
                let errPipe = Pipe()
                process.standardOutput = outPipe
                process.standardError = errPipe

                do {
                    try process.run()
                } catch {
                    continuation.resume(throwing: error)
                    return
                }

                let outBox = DataBox()
                let errBox = DataBox()
                let group = DispatchGroup()
                group.enter()
                DispatchQueue.global().async {
                    outBox.fill(from: outPipe.fileHandleForReading)
                    group.leave()
                }
                group.enter()
                DispatchQueue.global().async {
                    errBox.fill(from: errPipe.fileHandleForReading)
                    group.leave()
                }

                process.waitUntilExit()
                group.wait()  // ensures both reads are visible before we use the buffers

                if process.terminationStatus == 0 {
                    continuation.resume(returning: String(decoding: outBox.value, as: UTF8.self))
                } else {
                    continuation.resume(throwing: GitCommandError(
                        arguments: arguments,
                        exitCode: process.terminationStatus,
                        stderr: String(decoding: errBox.value, as: UTF8.self)
                    ))
                }
            }
        }
    }
}
