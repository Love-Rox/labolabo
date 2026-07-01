import XCTest
@testable import LaboLaboEngine

final class DiscoverReposTests: XCTestCase {
    private func makeRepo(_ url: URL) throws {
        try FileManager.default.createDirectory(
            at: url.appendingPathComponent(".git"), withIntermediateDirectories: true
        )
    }

    func testFindsNestedReposAndSkipsDescendingIntoThem() async throws {
        let tmp = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("disco-\(UUID().uuidString)")
        let repoA = tmp.appendingPathComponent("repoA")
        let repoB = tmp.appendingPathComponent("sub/repoB")
        try makeRepo(repoA)
        try makeRepo(repoB)
        // repoA 配下にさらに .git を含むディレクトリがあっても、repoA の中へは降りない。
        try makeRepo(repoA.appendingPathComponent("nested"))
        defer { try? FileManager.default.removeItem(at: tmp) }

        let git = GitEngine()
        let names = (await git.discoverRepos(under: tmp)).map { $0.lastPathComponent }.sorted()
        XCTAssertEqual(names, ["repoA", "repoB"])
    }

    func testRepoItselfReturnsJustItself() async throws {
        let tmp = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("disco-\(UUID().uuidString)")
        try makeRepo(tmp)
        defer { try? FileManager.default.removeItem(at: tmp) }

        let git = GitEngine()
        let repos = await git.discoverRepos(under: tmp)
        XCTAssertEqual(repos.map { $0.lastPathComponent }, [tmp.lastPathComponent])
    }
}
