import XCTest
@testable import LaboLaboEngine

/// `git` を偽の実行ファイルへすり替えるロケータ。GitRunner がロケータ解決パスを
/// 実際に使っていること（ハードコードされた `/usr/bin/env git` に戻っていないこと）
/// を確認するためのテスト用フェイク。
private enum FakeGitLocator: ToolLocating {
    static func locate(_ name: String) -> URL? {
        guard name == "git" else { return nil }
        return URL(fileURLWithPath: "/bin/echo")
    }
}

/// 何を渡しても解決に失敗するロケータ。「PATH に無い」場合のフォールバック
/// （`/usr/bin/env git`）が維持されていることを確認する。
private enum NeverResolvingLocator: ToolLocating {
    static func locate(_ name: String) -> URL? { nil }
}

final class GitRunnerTests: XCTestCase {

    private var dir: URL!

    override func setUpWithError() throws {
        dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("labolabo-gitrunner-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        if let dir { try? FileManager.default.removeItem(at: dir) }
    }

    func testRunReturnsStdout() async throws {
        let out = try await GitRunner.run(["--version"], in: dir)
        XCTAssertTrue(out.hasPrefix("git version"))
    }

    /// ロケータが解決した絶対パスを実際に起動していることを確認する。
    /// `git` を `/bin/echo` にすり替えると、渡した引数がそのまま echo される
    /// （`/usr/bin/env git` へフォールバックしていれば git のエラーになるはず）。
    func testRunUsesLocatorResolvedExecutable() async throws {
        let out = try await GitRunner.run(["hello-seam"], in: dir, locator: FakeGitLocator.self)
        XCTAssertEqual(out, "hello-seam\n")
    }

    /// ロケータが解決できない場合は、これまで通り `/usr/bin/env git` にフォールバック
    /// し、通常の git 呼び出しと同じ挙動を保つ。
    func testRunFallsBackToEnvGitWhenLocatorFails() async throws {
        let out = try await GitRunner.run(["--version"], in: dir, locator: NeverResolvingLocator.self)
        XCTAssertTrue(out.hasPrefix("git version"))
    }

    func testNonZeroExitThrowsGitCommandErrorWithStderr() async {
        // 非 repo ディレクトリでの rev-parse は非ゼロ exit + stderr を返す。
        do {
            try await GitRunner.run(["rev-parse", "--show-toplevel"], in: dir)
            XCTFail("expected GitCommandError")
        } catch let error as GitCommandError {
            XCTAssertNotEqual(error.exitCode, 0)
            XCTAssertFalse(error.stderr.isEmpty)
        } catch {
            XCTFail("unexpected error type: \(error)")
        }
    }

    /// 大量の並行 git 呼び出しがすべて完走すること。
    ///
    /// 旧実装（グローバルキューで waitUntilExit + group.wait）は呼び出しごとに
    /// GCD ワーカーを塞ぐため、並行数がプール上限（64）に達すると読み取り側が
    /// 永遠にスケジュールされずデッドロックした（アプリの終了ハングの根本原因）。
    func testManyConcurrentInvocationsAllComplete() async throws {
        let results = await withTaskGroup(of: Bool.self, returning: [Bool].self) { group in
            for _ in 0 ..< 100 {
                group.addTask { [dir = self.dir!] in
                    (try? await GitRunner.run(["--version"], in: dir)) != nil
                }
            }
            var collected: [Bool] = []
            for await ok in group { collected.append(ok) }
            return collected
        }
        XCTAssertEqual(results.count, 100)
        XCTAssertTrue(results.allSatisfy { $0 })
    }
}
