import XCTest
@testable import LaboLaboStore

final class SessionPersistingTests: XCTestCase {

    private var dbURL: URL!

    override func setUpWithError() throws {
        dbURL = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("labolabo-store-\(UUID().uuidString)")
            .appendingPathComponent("db.sqlite")
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dbURL.deletingLastPathComponent())
    }

    /// `SessionDatabase` は素通しではなく `SessionPersisting` 経由でも使えること
    /// （app 側が受け取る型を protocol に切り替えられることの回帰確認）。
    func testSessionDatabaseSatisfiesSessionPersisting() throws {
        let db: SessionPersisting = try SessionDatabase(url: dbURL)
        let record = SessionRecord(
            id: "seam-1", worktreePath: "/tmp/seam-1", name: "seam-1",
            addedAt: Date(), sortOrder: 0
        )
        try db.upsert(record)
        XCTAssertEqual(try db.allSessions().map(\.id), ["seam-1"])

        try db.setAppState("v", forKey: "k")
        XCTAssertEqual(try db.appState(forKey: "k"), "v")
        XCTAssertEqual(try db.appStateEntries(prefix: "k"), ["k": "v"])

        try db.setSelectedSessionID("seam-1")
        XCTAssertEqual(try db.selectedSessionID(), "seam-1")

        try db.deleteSession(id: "seam-1")
        XCTAssertEqual(try db.allSessions().count, 0)
    }

    /// データディレクトリ解決の集約先 `AppDataDirectory` が、従来 `defaultURL()` に
    /// 埋め込まれていたパスと完全に一致すること（挙動不変の確認）。
    func testDefaultURLMatchesAppDataDirectory() {
        let expected = AppDataDirectory.url().appendingPathComponent("labolabo.db")
        XCTAssertEqual(SessionDatabase.defaultURL(), expected)

        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        XCTAssertEqual(
            SessionDatabase.defaultURL(),
            base.appendingPathComponent("LaboLabo/labolabo.db"),
            "従来のパス生成（1回の appendingPathComponent）と同一でなければならない"
        )
    }
}
