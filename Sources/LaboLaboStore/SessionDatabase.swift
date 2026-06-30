import Foundation
import GRDB

/// SQLite-backed (GRDB) store for app-owned session metadata. Schema is versioned
/// via `DatabaseMigrator` so it can evolve (terminal layout, agent session ids…)
/// without losing data.
public final class SessionDatabase {
    private let dbQueue: DatabaseQueue

    public init(url: URL) throws {
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        dbQueue = try DatabaseQueue(path: url.path)
        try Self.migrator.migrate(dbQueue)
    }

    /// `~/Library/Application Support/LaboLabo/labolabo.db`
    public static func defaultURL() -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("LaboLabo/labolabo.db")
    }

    private static var migrator: DatabaseMigrator {
        var migrator = DatabaseMigrator()
        migrator.registerMigration("v1") { db in
            try db.create(table: "session") { t in
                t.primaryKey("id", .text)
                t.column("worktreePath", .text).notNull()
                t.column("name", .text).notNull()
                t.column("branch", .text)
                t.column("addedAt", .datetime).notNull()
                t.column("sortOrder", .integer).notNull().defaults(to: 0)
            }
            try db.create(table: "appState") { t in
                t.primaryKey("key", .text)
                t.column("value", .text)
            }
        }
        return migrator
    }

    // MARK: - Sessions

    public func allSessions() throws -> [SessionRecord] {
        try dbQueue.read { db in
            try SessionRecord
                .order(Column("sortOrder"))
                .fetchAll(db)
        }
    }

    public func upsert(_ record: SessionRecord) throws {
        try dbQueue.write { db in
            try record.save(db)
        }
    }

    public func deleteSession(id: String) throws {
        _ = try dbQueue.write { db in
            try SessionRecord.deleteOne(db, key: id)
        }
    }

    // MARK: - App state (e.g. last selection)

    public func setSelectedSessionID(_ id: String?) throws {
        try dbQueue.write { db in
            try db.execute(
                sql: """
                INSERT INTO appState(key, value) VALUES('selectedSession', ?)
                ON CONFLICT(key) DO UPDATE SET value = excluded.value
                """,
                arguments: [id]
            )
        }
    }

    public func selectedSessionID() throws -> String? {
        try dbQueue.read { db in
            try String.fetchOne(db, sql: "SELECT value FROM appState WHERE key = 'selectedSession'")
        }
    }
}
