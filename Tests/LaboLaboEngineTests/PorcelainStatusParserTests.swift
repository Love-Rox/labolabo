import XCTest
@testable import LaboLaboEngine

final class PorcelainStatusParserTests: XCTestCase {

    private func nulJoined(_ lines: [String]) -> String {
        lines.joined(separator: "\u{0}") + "\u{0}"
    }

    func testBranchHeadersAndAheadBehind() {
        let raw = nulJoined([
            "# branch.oid abc123",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
        ])
        let status = PorcelainStatusParser.parse(raw)
        XCTAssertEqual(status.headSha, "abc123")
        XCTAssertEqual(status.branch, "main")
        XCTAssertEqual(status.upstream, "origin/main")
        XCTAssertEqual(status.ahead, 2)
        XCTAssertEqual(status.behind, 1)
        XCTAssertFalse(status.isDetached)
    }

    func testDetachedHead() {
        let raw = nulJoined(["# branch.head (detached)"])
        XCTAssertTrue(PorcelainStatusParser.parse(raw).isDetached)
    }

    func testOrdinaryStagedUnstagedAndPathWithSpace() {
        let raw = nulJoined([
            "# branch.head main",
            "1 .M N... 100644 100644 100644 1111111 2222222 src/foo.swift",
            "1 M. N... 100644 100644 100644 3333333 4444444 src/bar baz.swift",
        ])
        let status = PorcelainStatusParser.parse(raw)
        XCTAssertEqual(status.entries.count, 2)

        let foo = status.entries[0]
        XCTAssertEqual(foo.path, "src/foo.swift")
        XCTAssertEqual(foo.index, .unmodified)
        XCTAssertEqual(foo.worktree, .modified)
        XCTAssertTrue(foo.isUnstaged)
        XCTAssertFalse(foo.isStaged)

        let bar = status.entries[1]
        XCTAssertEqual(bar.path, "src/bar baz.swift", "spaces in paths must be preserved")
        XCTAssertEqual(bar.index, .modified)
        XCTAssertTrue(bar.isStaged)

        XCTAssertEqual(status.staged.map(\.path), ["src/bar baz.swift"])
        XCTAssertEqual(status.unstaged.map(\.path), ["src/foo.swift"])
    }

    func testRenameConsumesOriginalPathToken() {
        let raw = nulJoined([
            "# branch.head main",
            "2 R. N... 100644 100644 100644 5555555 6666666 R100 new/name.swift",
            "old/name.swift",
            "? untracked.txt",
        ])
        let status = PorcelainStatusParser.parse(raw)
        XCTAssertEqual(status.entries.count, 2, "rename + untracked; the original-path token must not become its own entry")

        let rename = status.entries[0]
        XCTAssertEqual(rename.kind, .renamedOrCopied)
        XCTAssertEqual(rename.path, "new/name.swift")
        XCTAssertEqual(rename.originalPath, "old/name.swift")
        XCTAssertEqual(rename.score, 100)
        XCTAssertEqual(rename.index, .renamed)

        XCTAssertEqual(status.untracked.map(\.path), ["untracked.txt"])
    }

    func testCleanRepoIsNotDirty() {
        let raw = nulJoined(["# branch.head main", "# branch.ab +0 -0"])
        XCTAssertFalse(PorcelainStatusParser.parse(raw).isDirty)
    }
}
