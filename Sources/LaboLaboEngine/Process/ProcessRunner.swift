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
    ///
    /// `readDataToEndOfFile()` ではなく `read(upToCount:)` ループで読むのは、
    /// **別スレッドからハンドルを閉じて読みを中断できる**ようにするため。孫プロセスが
    /// パイプの write 端を握って EOF が来ないとき、呼び出し側が read ハンドルを close すると
    /// この read が throw して try? で握りつぶされ、ループが安全に抜ける（ハング回避）。
    func fill(from handle: FileHandle) {
        var data = Data()
        while let chunk = try? handle.read(upToCount: 1 << 16), !chunk.isEmpty {
            data.append(chunk)
        }
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
        let timedOut = exited.wait(timeout: .now() + timeout) == .timedOut
        if timedOut {
            process.terminate()
            if exited.wait(timeout: .now() + 1) == .timedOut {
                kill(process.processIdentifier, SIGKILL)
                exited.wait()
            }
        }

        // 子プロセスが終了しても、孫プロセスがパイプの write 端を握っていると
        // read が EOF を受け取れず drain が永久に止まる（例: ログインシェルが profile で
        // バックグラウンド常駐を起動するケース。CI で `swift test` がハングする原因だった）。
        // 子の終了後に少し待っても drain が終わらなければ、read ハンドルを閉じて強制的に解く
        // （読めた分だけを採用する）。
        if drain.wait(timeout: .now() + 2) == .timedOut {
            try? outPipe.fileHandleForReading.close()
            try? errPipe.fileHandleForReading.close()
            drain.wait()
        }

        if timedOut { return nil }
        return Output(
            status: process.terminationStatus,
            stdout: String(decoding: outBox.value, as: UTF8.self),
            stderr: String(decoding: errBox.value, as: UTF8.self)
        )
    }
}
