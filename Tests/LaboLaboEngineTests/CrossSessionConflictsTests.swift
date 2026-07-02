import XCTest
@testable import LaboLaboEngine

final class CrossSessionConflictsTests: XCTestCase {

    private typealias S = CrossSessionConflicts.Session

    func testDetectsSharedPathInSameRepo() {
        let sessions = [
            S(id: "a", repoKey: "R", changed: ["src/foo.swift", "a.txt"]),
            S(id: "b", repoKey: "R", changed: ["src/foo.swift", "b.txt"]),
        ]
        let conflicts = CrossSessionConflicts.conflicts(for: "a", among: sessions)
        XCTAssertEqual(conflicts, [.init(path: "src/foo.swift", others: ["b"])])
    }

    func testNoConflictAcrossDifferentRepos() {
        // 同じパスでも repoKey が違えば衝突ではない。
        let sessions = [
            S(id: "a", repoKey: "R1", changed: ["foo.swift"]),
            S(id: "b", repoKey: "R2", changed: ["foo.swift"]),
        ]
        XCTAssertTrue(CrossSessionConflicts.conflicts(for: "a", among: sessions).isEmpty)
    }

    func testNoConflictWhenAlone() {
        let sessions = [S(id: "a", repoKey: "R", changed: ["foo.swift"])]
        XCTAssertTrue(CrossSessionConflicts.conflicts(for: "a", among: sessions).isEmpty)
    }

    func testUnresolvedRepoKeyIsIgnored() {
        // repoKey 未解決（nil）は同居判定に含めない。
        let sessions = [
            S(id: "a", repoKey: nil, changed: ["foo.swift"]),
            S(id: "b", repoKey: nil, changed: ["foo.swift"]),
        ]
        XCTAssertTrue(CrossSessionConflicts.conflicts(for: "a", among: sessions).isEmpty)
    }

    func testMultipleOthersAndSortedPaths() {
        let sessions = [
            S(id: "a", repoKey: "R", changed: ["z.swift", "a.swift"]),
            S(id: "b", repoKey: "R", changed: ["a.swift"]),
            S(id: "c", repoKey: "R", changed: ["a.swift", "z.swift"]),
        ]
        let conflicts = CrossSessionConflicts.conflicts(for: "a", among: sessions)
        // パスは昇順、others は入力順（b, c）。
        XCTAssertEqual(conflicts, [
            .init(path: "a.swift", others: ["b", "c"]),
            .init(path: "z.swift", others: ["c"]),
        ])
    }

    func testEmptyChangedSetHasNoConflict() {
        let sessions = [
            S(id: "a", repoKey: "R", changed: []),
            S(id: "b", repoKey: "R", changed: ["foo.swift"]),
        ]
        XCTAssertTrue(CrossSessionConflicts.conflicts(for: "a", among: sessions).isEmpty)
    }
}
