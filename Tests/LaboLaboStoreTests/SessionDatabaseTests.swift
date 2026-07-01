import XCTest
@testable import LaboLaboStore

final class SessionDatabaseTests: XCTestCase {

    private var dbURL: URL!

    override func setUpWithError() throws {
        dbURL = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("labolabo-store-\(UUID().uuidString)")
            .appendingPathComponent("db.sqlite")
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dbURL.deletingLastPathComponent())
    }

    private func record(_ id: String, order: Int, branch: String? = nil) -> SessionRecord {
        SessionRecord(
            id: id,
            worktreePath: "/tmp/\(id)",
            name: id,
            branch: branch,
            addedAt: Date(timeIntervalSince1970: TimeInterval(order)),
            sortOrder: order
        )
    }

    func testUpsertFetchDelete() throws {
        let db = try SessionDatabase(url: dbURL)
        XCTAssertEqual(try db.allSessions().count, 0)

        try db.upsert(record("repo-1", order: 0, branch: "main"))
        let fetched = try db.allSessions()
        XCTAssertEqual(fetched.count, 1)
        XCTAssertEqual(fetched[0].id, "repo-1")
        XCTAssertEqual(fetched[0].branch, "main")

        try db.upsert(record("repo-1", order: 0, branch: "feature/x"))
        XCTAssertEqual(try db.allSessions().count, 1, "same id must update, not insert")
        XCTAssertEqual(try db.allSessions().first?.branch, "feature/x")

        try db.deleteSession(id: "repo-1")
        XCTAssertEqual(try db.allSessions().count, 0)
    }

    func testAgentSessionRoundTrip() throws {
        let db = try SessionDatabase(url: dbURL)
        var rec = record("repo-1", order: 0, branch: "main")
        rec.agentSessionId = "sess-abc-123"
        rec.transcriptPath = "/tmp/transcript.jsonl"
        try db.upsert(rec)

        let fetched = try XCTUnwrap(try db.allSessions().first)
        XCTAssertEqual(fetched.agentSessionId, "sess-abc-123")
        XCTAssertEqual(fetched.transcriptPath, "/tmp/transcript.jsonl")

        // 既定は nil（後方互換）。
        try db.upsert(record("repo-2", order: 1))
        let plain = try XCTUnwrap(try db.allSessions().first { $0.id == "repo-2" })
        XCTAssertNil(plain.agentSessionId)
        XCTAssertNil(plain.transcriptPath)
    }

    func testOrdering() throws {
        let db = try SessionDatabase(url: dbURL)
        try db.upsert(record("b", order: 1))
        try db.upsert(record("a", order: 0))
        XCTAssertEqual(try db.allSessions().map(\.id), ["a", "b"])
    }

    func testSelectedSessionID() throws {
        let db = try SessionDatabase(url: dbURL)
        XCTAssertNil(try db.selectedSessionID())
        try db.setSelectedSessionID("id-9")
        XCTAssertEqual(try db.selectedSessionID(), "id-9")
        try db.setSelectedSessionID(nil)
        XCTAssertNil(try db.selectedSessionID())
    }

    func testPersistenceAcrossInstances() throws {
        do {
            let db = try SessionDatabase(url: dbURL)
            try db.upsert(record("p", order: 0))
            try db.setSelectedSessionID("p")
        }
        let reopened = try SessionDatabase(url: dbURL)
        XCTAssertEqual(try reopened.allSessions().map(\.id), ["p"])
        XCTAssertEqual(try reopened.selectedSessionID(), "p")
    }
}
