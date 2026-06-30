import XCTest
@testable import LaboLaboEngine

/// Exercises GitEngine against a real `git` binary in a throwaway repo.
final class GitEngineIntegrationTests: XCTestCase {

    private var repo: URL!

    override func setUpWithError() throws {
        repo = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("labolabo-git-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: repo, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        if let repo { try? FileManager.default.removeItem(at: repo) }
    }

    private func git(_ args: [String]) async throws {
        try await GitRunner.run(args, in: repo)
    }

    private func write(_ name: String, _ content: String) throws {
        try content.write(to: repo.appendingPathComponent(name), atomically: true, encoding: .utf8)
    }

    private func initRepoWithCommit() async throws {
        try await git(["init", "-b", "main"])
        try await git(["config", "user.email", "test@example.com"])
        try await git(["config", "user.name", "LaboLabo Test"])
        try write("a.txt", "one\ntwo\nthree\n")
        try await git(["add", "."])
        try await git(["-c", "commit.gpgsign=false", "commit", "-m", "init"])
    }

    func testStatusDiffNumstatAndFileContents() async throws {
        try await initRepoWithCommit()
        // Modify a tracked file and add an untracked one.
        try write("a.txt", "one\ntwo changed\nthree\nfour\n")
        try write("b.txt", "new file\n")

        let engine = GitEngine()

        let status = try await engine.status(worktree: repo)
        XCTAssertEqual(status.branch, "main")
        XCTAssertTrue(status.isDirty)
        XCTAssertTrue(status.unstaged.contains { $0.path == "a.txt" })
        XCTAssertEqual(status.untracked.map(\.path), ["b.txt"])

        let diffs = try await engine.diff(worktree: repo)
        let aDiff = try XCTUnwrap(diffs.first { $0.displayPath == "a.txt" })
        XCTAssertGreaterThanOrEqual(aDiff.additions, 1)
        XCTAssertGreaterThanOrEqual(aDiff.deletions, 1)

        let single = try await engine.diff(worktree: repo, path: "a.txt")
        XCTAssertEqual(single?.displayPath, "a.txt")

        let numstat = try await engine.numstat(worktree: repo)
        XCTAssertTrue(numstat.contains { $0.path == "a.txt" })

        XCTAssertEqual(try engine.fileContents(worktree: repo, path: "b.txt"), "new file\n")
    }

    func testWorktreeAddListRemove() async throws {
        try await initRepoWithCommit()

        let engine = GitEngine()
        let wtPath = repo.appendingPathComponent(".worktrees/feature-x")
        try await engine.addWorktree(repo: repo, path: wtPath, branch: "feature/x", baseRef: "main")

        let listed = try await engine.listWorktrees(repo: repo)
        XCTAssertTrue(listed.contains { $0.shortBranch == "feature/x" })

        try await engine.removeWorktree(repo: repo, path: wtPath, force: true)
        let after = try await engine.listWorktrees(repo: repo)
        XCTAssertFalse(after.contains { $0.shortBranch == "feature/x" })
    }
}
