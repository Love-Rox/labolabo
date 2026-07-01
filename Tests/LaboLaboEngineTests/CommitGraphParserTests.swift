import XCTest
@testable import LaboLaboEngine

final class CommitGraphParserTests: XCTestCase {

    private let us = "\u{1f}"

    /// Build one raw log line in the `%H %h %s %an %at %P %d` layout.
    private func line(
        full: String, short: String, subject: String, author: String,
        at: Int, parents: [String], refs: String = ""
    ) -> String {
        [full, short, subject, author, "\(at)", parents.joined(separator: " "), refs]
            .joined(separator: us)
    }

    private func shapes(_ row: CommitGraphRow, _ shape: CommitGraphRow.Edge.Shape) -> [Int] {
        row.edges.filter { $0.shape == shape }.map(\.lane).sorted()
    }

    func testParsesCommitFields() throws {
        let raw = line(
            full: "abc1234def", short: "abc1234", subject: "feat: hello",
            author: "Alice", at: 1_700_000_000, parents: [], refs: " (HEAD -> main, origin/main)"
        )
        let rows = CommitGraphLayout.build(raw)
        XCTAssertEqual(rows.count, 1)
        let c = rows[0].commit
        XCTAssertEqual(c.hash, "abc1234")
        XCTAssertEqual(c.subject, "feat: hello")
        XCTAssertEqual(c.author, "Alice")
        XCTAssertEqual(c.date, Date(timeIntervalSince1970: 1_700_000_000))
        XCTAssertEqual(c.refs, "HEAD -> main, origin/main")
        // 親も子もない単独コミット: ノードは lane 0、エッジ無し。
        XCTAssertEqual(rows[0].nodeLane, 0)
        XCTAssertTrue(rows[0].edges.isEmpty)
    }

    func testLinearHistoryStaysInOneLane() {
        let raw = [
            line(full: "A", short: "A", subject: "a", author: "X", at: 2, parents: ["B"]),
            line(full: "B", short: "B", subject: "b", author: "X", at: 1, parents: []),
        ].joined(separator: "\n")
        let rows = CommitGraphLayout.build(raw)
        XCTAssertEqual(rows.count, 2)
        // A は最初の親 B へ下向き（nodeOut lane0）。
        XCTAssertEqual(rows[0].nodeLane, 0)
        XCTAssertEqual(shapes(rows[0], .nodeOut), [0])
        // B は上から入るだけ（nodeIn lane0）。どちらも lane 0 の直線。
        XCTAssertEqual(rows[1].nodeLane, 0)
        XCTAssertEqual(shapes(rows[1], .nodeIn), [0])
    }

    func testMergeUsesStableLanes() {
        // m は親 a(第1)/b(第2) のマージ。b は a を親に持つ feature 1 コミット。
        let raw = [
            line(full: "m", short: "m", subject: "merge", author: "X", at: 3, parents: ["a", "b"]),
            line(full: "b", short: "b", subject: "feat", author: "X", at: 2, parents: ["a"]),
            line(full: "a", short: "a", subject: "init", author: "X", at: 1, parents: []),
        ].joined(separator: "\n")
        let rows = CommitGraphLayout.build(raw)
        XCTAssertEqual(rows.count, 3)

        // マージコミット: lane0=第1親 a、lane1=第2親 b へ 2 本の nodeOut。
        XCTAssertEqual(rows[0].nodeLane, 0)
        XCTAssertEqual(shapes(rows[0], .nodeOut), [0, 1])

        // feature コミット b: lane0 は a を待って通過、自身は lane1。
        XCTAssertEqual(rows[1].nodeLane, 1)
        XCTAssertEqual(shapes(rows[1], .through), [0])
        XCTAssertEqual(shapes(rows[1], .nodeIn), [1])
        XCTAssertEqual(shapes(rows[1], .nodeOut), [1])

        // 分岐元 a: lane0 と lane1 の 2 本が合流（nodeIn 2 本）。
        XCTAssertEqual(rows[2].nodeLane, 0)
        XCTAssertEqual(shapes(rows[2], .nodeIn), [0, 1])
    }
}
