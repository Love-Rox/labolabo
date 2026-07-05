import XCTest
@testable import LaboLaboEngine

final class ProcessRunnerTests: XCTestCase {

    // MARK: - Fixtures

    /// 基本システムバイナリのみ利用（ハーメティックに保つ）。
    private let echo = URL(fileURLWithPath: "/bin/echo")
    private let sh = URL(fileURLWithPath: "/bin/sh")

    // MARK: - stdout / status

    func testEchoCapturesStdoutAndZeroStatus() throws {
        let out = try XCTUnwrap(
            ProcessRunner.runSync(executable: echo, arguments: ["hello"])
        )
        XCTAssertEqual(out.status, 0)
        // /bin/echo は末尾に改行を付けるが、呼び出し側で trim して照合する。
        XCTAssertEqual(out.stdout.trimmingCharacters(in: .whitespacesAndNewlines), "hello")
        XCTAssertTrue(out.stderr.isEmpty)
    }

    func testEchoRawStdoutIncludesTrailingNewline() throws {
        let out = try XCTUnwrap(
            ProcessRunner.runSync(executable: echo, arguments: ["abc"])
        )
        // trim 前の生 stdout は改行付き（drain がバイト列をそのまま返すことの確認）。
        XCTAssertEqual(out.stdout, "abc\n")
    }

    // MARK: - 終了ステータス

    func testExitCodeIsPropagated() throws {
        let out = try XCTUnwrap(
            ProcessRunner.runSync(executable: sh, arguments: ["-c", "exit 3"])
        )
        XCTAssertEqual(out.status, 3)
        XCTAssertTrue(out.stdout.isEmpty)
        XCTAssertTrue(out.stderr.isEmpty)
    }

    // MARK: - stderr

    func testStderrIsCapturedSeparatelyFromStdout() throws {
        let out = try XCTUnwrap(
            ProcessRunner.runSync(executable: sh, arguments: ["-c", "echo err 1>&2"])
        )
        XCTAssertEqual(out.status, 0)
        // stderr に流したものは stdout に混ざらない。
        XCTAssertEqual(out.stderr.trimmingCharacters(in: .whitespacesAndNewlines), "err")
        XCTAssertTrue(out.stdout.isEmpty)
    }

    func testStdoutAndStderrDrainedIndependently() throws {
        let out = try XCTUnwrap(
            ProcessRunner.runSync(
                executable: sh,
                arguments: ["-c", "echo out; echo bad 1>&2"]
            )
        )
        XCTAssertEqual(out.status, 0)
        XCTAssertEqual(out.stdout.trimmingCharacters(in: .whitespacesAndNewlines), "out")
        XCTAssertEqual(out.stderr.trimmingCharacters(in: .whitespacesAndNewlines), "bad")
    }

    // MARK: - working directory

    func testRunsInSpecifiedDirectory() throws {
        let base = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: base) }

        let out = try XCTUnwrap(
            ProcessRunner.runSync(executable: sh, arguments: ["-c", "pwd"], in: base)
        )
        XCTAssertEqual(out.status, 0)
        // symlink（/private 等）差異を吸収するため realpath 同士で比較。
        let reported = URL(fileURLWithPath: out.stdout.trimmingCharacters(in: .whitespacesAndNewlines))
            .resolvingSymlinksInPath().path
        XCTAssertEqual(reported, base.resolvingSymlinksInPath().path)
    }

    // MARK: - environment

    func testEnvironmentIsPassedThrough() throws {
        let out = try XCTUnwrap(
            ProcessRunner.runSync(
                executable: sh,
                arguments: ["-c", "printf %s \"$LABO_TEST_VAR\""],
                environment: ["LABO_TEST_VAR": "wired"]
            )
        )
        XCTAssertEqual(out.status, 0)
        XCTAssertEqual(out.stdout, "wired")
    }

    // MARK: - timeout

    func testTimeoutReturnsNilWhenCommandOutlivesDeadline() {
        // 5 秒 sleep を 1 秒 timeout で打ち切る → nil。
        let out = ProcessRunner.runSync(
            executable: sh,
            arguments: ["-c", "sleep 5"],
            timeout: 1
        )
        XCTAssertNil(out)
    }

    func testFastCommandCompletesWithinTimeout() throws {
        // timeout 以内に終わるコマンドは nil にならず結果を返す。
        let out = try XCTUnwrap(
            ProcessRunner.runSync(executable: sh, arguments: ["-c", "exit 0"], timeout: 5)
        )
        XCTAssertEqual(out.status, 0)
    }

    // MARK: - 起動失敗

    func testMissingExecutableReturnsNil() {
        let missing = URL(fileURLWithPath: "/nonexistent/definitely/not/here-\(UUID().uuidString)")
        XCTAssertNil(ProcessRunner.runSync(executable: missing, arguments: []))
    }

    // MARK: - 孫プロセスがパイプを握ってもハングしない（CI ハングの回帰テスト）

    /// 子（sh）は即終了するが、孫（`sleep &`）が stdout を継承したまま生き残ると、
    /// 素朴な `readDataToEndOfFile()` は EOF を受け取れず**永久にハング**する。
    /// これは CI（ログインシェルが profile で常駐を起動する環境）で `swift test` が
    /// 止まっていた原因。drain を有限時間で打ち切ることで、読めた分を返しつつ有限で戻る。
    func testDoesNotHangWhenGrandchildKeepsPipeOpen() throws {
        let start = Date()
        let out = try XCTUnwrap(
            ProcessRunner.runSync(
                executable: sh,
                // echo で stdout に書いた後、sleep をバックグラウンド起動して stdout を握らせる。
                arguments: ["-c", "echo hi; sleep 10 &"],
                timeout: 20
            )
        )
        let elapsed = Date().timeIntervalSince(start)
        XCTAssertEqual(out.status, 0)
        XCTAssertTrue(out.stdout.contains("hi"), "子の出力は取得できるべき")
        // 孫の sleep(10) を待たずに、drain 強制解除（~2s）で有限に戻ること。
        XCTAssertLessThan(elapsed, 6, "孫がパイプを握っていても有限時間で返るべき")
    }
}
