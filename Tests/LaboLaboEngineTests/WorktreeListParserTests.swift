import XCTest
@testable import LaboLaboEngine

final class WorktreeListParserTests: XCTestCase {

    func testParsesBlocks() {
        let raw = """
        worktree /repo
        HEAD aaaaaaa
        branch refs/heads/main

        worktree /repo/.worktrees/x
        HEAD bbbbbbb
        branch refs/heads/feature/x

        worktree /repo/locked-detached
        HEAD ccccccc
        detached
        locked
        """
        let worktrees = WorktreeListParser.parse(raw)
        XCTAssertEqual(worktrees.count, 3)

        XCTAssertEqual(worktrees[0].path, "/repo")
        XCTAssertEqual(worktrees[0].shortBranch, "main")
        XCTAssertFalse(worktrees[0].isDetached)

        XCTAssertEqual(worktrees[1].shortBranch, "feature/x")

        XCTAssertTrue(worktrees[2].isDetached)
        XCTAssertTrue(worktrees[2].isLocked)
        XCTAssertNil(worktrees[2].shortBranch)
    }
}
