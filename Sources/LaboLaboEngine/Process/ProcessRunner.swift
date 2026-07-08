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

    /// チャンクを追記する（read source からの逐次読み用）。
    func append(_ chunk: Data) {
        lock.lock()
        storage.append(chunk)
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

    /// 全 run() の I/O・回収イベントを直列化する専用キュー。ハンドラは
    /// チャンクの append 程度なので 1 本で十分。
    private static let ioQueue = DispatchQueue(label: "labolabo.process-runner")

    /// コマンドを非同期実行し `(status, stdout, stderr)` を返す。
    ///
    /// `runSync` と違い、待機中にスレッドを 1 本も占有しない: `posix_spawn` で
    /// 起動し、stdout/stderr は read の DispatchSource、exit は proc の
    /// DispatchSource + `waitpid` で受け、3 つの完了が揃ってから再開する。
    /// GCD ワーカープール（ソフト上限 64）を消費しないので、大量に並行実行
    /// してもプール枯渇デッドロックを起こさない。
    ///
    /// NSTask（`Process`）をあえて使わない: `terminationHandler` /
    /// `readabilityHandler` ベースの実装は、数十件の並行実行後に同一プロセス内の
    /// 別 NSTask の終了通知が届かなくなる現象を macOS 26.5 で起こした
    /// （`waitUntilExit` が永遠に返らない）。カーネル API 直叩きなら
    /// Foundation の共有状態に触れない。
    ///
    /// - Throws: 起動に失敗したときのみ。非ゼロ exit は `Output.status` で返す
    ///   （シグナル死はシェル慣習の 128+signo に写像する）。
    public static func run(
        executable: URL,
        arguments: [String],
        in directory: URL? = nil,
        environment: [String: String]? = nil
    ) async throws -> Output {
        var outFDs: [Int32] = [-1, -1] // [read, write]
        var errFDs: [Int32] = [-1, -1]
        guard pipe(&outFDs) == 0 else {
            throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
        }
        guard pipe(&errFDs) == 0 else {
            let code = errno
            close(outFDs[0]); close(outFDs[1])
            throw POSIXError(POSIXErrorCode(rawValue: code) ?? .EIO)
        }

        var fileActions: posix_spawn_file_actions_t?
        posix_spawn_file_actions_init(&fileActions)
        defer { posix_spawn_file_actions_destroy(&fileActions) }
        // CLOEXEC_DEFAULT で fd 0/1/2 も閉じられるので、stdin は /dev/null を明示。
        posix_spawn_file_actions_addopen(&fileActions, 0, "/dev/null", O_RDONLY, 0)
        posix_spawn_file_actions_adddup2(&fileActions, outFDs[1], 1)
        posix_spawn_file_actions_adddup2(&fileActions, errFDs[1], 2)
        if let directory {
            posix_spawn_file_actions_addchdir_np(&fileActions, directory.path)
        }

        var attrs: posix_spawnattr_t?
        posix_spawnattr_init(&attrs)
        defer { posix_spawnattr_destroy(&attrs) }
        // 並行 spawn 中の兄弟プロセスへ pipe write 端が漏れると EOF が来なくなる。
        posix_spawnattr_setflags(&attrs, Int16(POSIX_SPAWN_CLOEXEC_DEFAULT))

        let argv = ([executable.path] + arguments).map { strdup($0) } + [nil]
        let env = environment ?? ProcessInfo.processInfo.environment
        let envp = env.map { strdup("\($0.key)=\($0.value)") } + [nil]
        defer {
            argv.forEach { free($0) }
            envp.forEach { free($0) }
        }

        var pid: pid_t = 0
        let rc = posix_spawn(&pid, executable.path, &fileActions, &attrs, argv, envp)
        close(outFDs[1])
        close(errFDs[1])
        guard rc == 0 else {
            close(outFDs[0]); close(errFDs[0])
            throw POSIXError(POSIXErrorCode(rawValue: rc) ?? .EIO)
        }

        let outBox = DataBox()
        let errBox = DataBox()
        let status = ExitStatusBox()
        let done = DispatchGroup()
        done.enter() // stdout EOF
        done.enter() // stderr EOF
        done.enter() // exit 回収
        drain(fd: outFDs[0], into: outBox, done: done)
        drain(fd: errFDs[0], into: errBox, done: done)
        watchExit(pid: pid, status: status, done: done)

        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            done.notify(queue: ioQueue) { continuation.resume() }
        }
        return Output(
            status: status.value,
            stdout: String(decoding: outBox.value, as: UTF8.self),
            stderr: String(decoding: errBox.value, as: UTF8.self)
        )
    }

    /// `fd` を EOF まで非同期に読み、閉じてから `done.leave()` する。
    private static func drain(fd: Int32, into box: DataBox, done: DispatchGroup) {
        _ = fcntl(fd, F_SETFL, O_NONBLOCK)
        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: ioQueue)
        source.setEventHandler {
            var buffer = [UInt8](repeating: 0, count: 65536)
            while true {
                let n = read(fd, &buffer, buffer.count)
                if n > 0 {
                    box.append(Data(bytes: buffer, count: n))
                } else if n == 0 {
                    source.cancel()
                    return
                } else if errno == EINTR {
                    continue
                } else if errno == EAGAIN || errno == EWOULDBLOCK {
                    return
                } else {
                    source.cancel()
                    return
                }
            }
        }
        source.setCancelHandler {
            close(fd)
            done.leave()
        }
        source.resume()
    }

    /// 子プロセスの終了を proc source で監視し、`waitpid` で回収して
    /// `done.leave()` する。
    ///
    /// NOTE_EXIT はエッジイベントで、kqueue への登録は resume 後に非同期で
    /// 行われる。「登録完了前に exit」した子の分は発火しないので、
    /// (1) 登録完了時点の再チェック（registration handler）と
    /// (2) フォールバックの定期 `WNOHANG` ポーリングで必ず回収する。
    private static func watchExit(pid: pid_t, status: ExitStatusBox, done: DispatchGroup) {
        let source = DispatchSource.makeProcessSource(identifier: pid, eventMask: .exit, queue: ioQueue)
        let fallback = DispatchSource.makeTimerSource(queue: ioQueue)
        let reap = { @Sendable in
            var raw: Int32 = 0
            guard waitpid(pid, &raw, WNOHANG) == pid else { return }
            let exited = (raw & 0x7f) == 0
            status.set(exited ? (raw >> 8) & 0xff : 128 + (raw & 0x7f))
            source.cancel()
            fallback.cancel()
            done.leave()
        }
        source.setEventHandler(handler: reap)
        source.setRegistrationHandler(handler: reap)
        source.resume()
        fallback.schedule(deadline: .now() + .milliseconds(500), repeating: .milliseconds(500))
        fallback.setEventHandler(handler: reap)
        fallback.resume()
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

/// `waitpid` の結果をロック越しに受け渡す箱。
private final class ExitStatusBox: @unchecked Sendable {
    private let lock = NSLock()
    private var status: Int32 = -1

    func set(_ value: Int32) {
        lock.lock()
        status = value
        lock.unlock()
    }

    var value: Int32 {
        lock.lock()
        defer { lock.unlock() }
        return status
    }
}
