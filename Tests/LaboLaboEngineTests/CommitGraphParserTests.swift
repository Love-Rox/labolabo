import XCTest
@testable import LaboLaboEngine

final class CommitGraphParserTests: XCTestCase {

    private let us = "\u{1f}"

    func testParsesCommitLineWithRefs() throws {
        let raw = "*\(us)abc1234\(us)feat: hello\(us)Alice\(us)2 hours ago\(us) (HEAD -> main, origin/main)"
        let lines = CommitGraphParser.parse(raw)
        XCTAssertEqual(lines.count, 1)
        XCTAssertEqual(lines[0].graph, "*")
        let commit = try XCTUnwrap(lines[0].commit)
        XCTAssertEqual(commit.hash, "abc1234")
        XCTAssertEqual(commit.subject, "feat: hello")
        XCTAssertEqual(commit.author, "Alice")
        XCTAssertEqual(commit.relativeDate, "2 hours ago")
        XCTAssertEqual(commit.refs, "HEAD -> main, origin/main")
    }

    func testParsesCommitLineWithoutRefs() {
        let raw = "* \(us)deadbee\(us)fix: bug\(us)Bob\(us)3 days ago\(us)"
        let lines = CommitGraphParser.parse(raw)
        XCTAssertEqual(lines.count, 1)
        XCTAssertEqual(lines[0].graph, "* ")
        XCTAssertEqual(lines[0].commit?.hash, "deadbee")
        XCTAssertEqual(lines[0].commit?.refs, "")
    }

    func testConnectorLineHasNoCommit() {
        let raw = """
        *\(us)aaa\(us)s\(us)A\(us)now\(us)
        |\\
        | *\(us)bbb\(us)s2\(us)B\(us)now\(us)
        """
        let lines = CommitGraphParser.parse(raw)
        XCTAssertEqual(lines.count, 3)
        XCTAssertNotNil(lines[0].commit)
        XCTAssertNil(lines[1].commit)
        XCTAssertEqual(lines[1].graph, "|\\")
        XCTAssertEqual(lines[2].commit?.hash, "bbb")
    }
}
