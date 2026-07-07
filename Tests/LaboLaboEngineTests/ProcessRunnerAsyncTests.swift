import XCTest
@testable import LaboLaboEngine

/// `ProcessRunner.run`（非同期・スレッド非占有版）の契約を検証する。
final class ProcessRunnerAsyncTests: XCTestCase {

    private let echo = URL(fileURLWithPath: "/bin/echo")
    private let sh = URL(fileURLWithPath: "/bin/sh")

    func testEchoCapturesStdoutAndZeroStatus() async throws {
        let out = try await ProcessRunner.run(executable: echo, arguments: ["hello"])
        XCTAssertEqual(out.status, 0)
        XCTAssertEqual(out.stdout, "hello\n")
        XCTAssertTrue(out.stderr.isEmpty)
    }

    func testNonZeroExitAndStderrArePropagated() async throws {
        let out = try await ProcessRunner.run(
            executable: sh, arguments: ["-c", "echo bad 1>&2; exit 3"]
        )
        XCTAssertEqual(out.status, 3)
        XCTAssertTrue(out.stdout.isEmpty)
        XCTAssertEqual(out.stderr.trimmingCharacters(in: .whitespacesAndNewlines), "bad")
    }

    func testEmptyOutputCompletes() async throws {
        let out = try await ProcessRunner.run(executable: sh, arguments: ["-c", "exit 0"])
        XCTAssertEqual(out.status, 0)
        XCTAssertTrue(out.stdout.isEmpty)
        XCTAssertTrue(out.stderr.isEmpty)
    }

    /// パイプバッファ（~64KB）を大きく超える出力を両パイプ同時に流しても
    /// 取りこぼし・詰まりなく EOF まで読み切れること。
    func testLargeOutputOnBothPipes() async throws {
        let size = 300_000
        let out = try await ProcessRunner.run(
            executable: sh,
            arguments: ["-c", "yes a | head -c \(size); yes b | head -c \(size) 1>&2"]
        )
        XCTAssertEqual(out.status, 0)
        XCTAssertEqual(out.stdout.utf8.count, size)
        XCTAssertEqual(out.stderr.utf8.count, size)
    }

    func testSignalDeathMapsToShellConvention() async throws {
        // 自分に SIGKILL（遅延不可能）→ 128+9=137（シェル慣習）に写像される。
        let out = try await ProcessRunner.run(
            executable: sh, arguments: ["-c", "kill -9 $$"]
        )
        XCTAssertEqual(out.status, 137)
    }

    func testMissingExecutableThrows() async {
        let missing = URL(fileURLWithPath: "/nonexistent/definitely/not/here-\(UUID().uuidString)")
        do {
            _ = try await ProcessRunner.run(executable: missing, arguments: [])
            XCTFail("expected launch failure to throw")
        } catch {
            // 起動失敗のみ throw、が契約。
        }
    }
}
