import Foundation

/// 並行 drain の結果をロック越しに受け渡す箱。
///
/// `DispatchQueue.global().async` の中でパイプを EOF まで読み、その `Data` を
/// 呼び出し側へ返すための Sendable な入れ物。ロックで happens-before を保証するので、
/// Swift 6 の「並行実行クロージャでの captured var 変異」警告を安全に回避できる。
final class DataBox: @unchecked Sendable {
    private let lock = NSLock()
    private var storage = Data()

    /// ハンドルを EOF まで読み、結果を格納する（バックグラウンドで 1 回だけ呼ぶ）。
    func fill(from handle: FileHandle) {
        let data = handle.readDataToEndOfFile()
        lock.lock()
        storage = data
        lock.unlock()
    }

    var value: Data {
        lock.lock()
        defer { lock.unlock() }
        return storage
    }
}

/// コマンドを同期実行し `(status, stdout, stderr)` を返す小さなヘルパ。
///
/// stdout / stderr を**並行して drain** するのでパイプバッファ（~64KB）が満杯でも
/// デッドロックしない。`timeout` を超えたら terminate → SIGKILL で確実に回収し `nil` を返す。
/// GUI から `Task.detached` などバックグラウンドで呼ぶ前提（`waitUntilExit` で block するため）。
public enum ProcessRunner {

    public struct Output: Sendable {
        public let status: Int32
        public let stdout: String
        public let stderr: String
    }

    /// - Returns: 起動に失敗、または `timeout` 超過なら `nil`。
    public static func runSync(
        executable: URL,
        arguments: [String],
        in directory: URL? = nil,
        environment: [String: String]? = nil,
        timeout: TimeInterval = 10
    ) -> Output? {
        let process = Process()
        process.executableURL = executable
        process.arguments = arguments
        if let directory { process.currentDirectoryURL = directory }
        if let environment { process.environment = environment }

        let outPipe = Pipe()
        let errPipe = Pipe()
        process.standardOutput = outPipe
        process.standardError = errPipe

        do { try process.run() } catch { return nil }

        // 両パイプを並行 drain（片方だけ読むとバッファ満杯で相手が block しデッドロック）。
        let outBox = DataBox()
        let errBox = DataBox()
        let drain = DispatchGroup()
        drain.enter()
        DispatchQueue.global().async {
            outBox.fill(from: outPipe.fileHandleForReading)
            drain.leave()
        }
        drain.enter()
        DispatchQueue.global().async {
            errBox.fill(from: errPipe.fileHandleForReading)
            drain.leave()
        }

        // exit を別スレッドで待ち、timeout を掛ける。
        let exited = DispatchGroup()
        exited.enter()
        DispatchQueue.global().async {
            process.waitUntilExit()
            exited.leave()
        }
        if exited.wait(timeout: .now() + timeout) == .timedOut {
            process.terminate()
            if exited.wait(timeout: .now() + 1) == .timedOut {
                kill(process.processIdentifier, SIGKILL)
                exited.wait()
            }
            drain.wait() // パイプが閉じるので drain も完了する
            return nil
        }
        drain.wait()
        return Output(
            status: process.terminationStatus,
            stdout: String(decoding: outBox.value, as: UTF8.self),
            stderr: String(decoding: errBox.value, as: UTF8.self)
        )
    }
}
