import Foundation

/// The persistence operations the app actually uses, decoupled from the concrete
/// storage engine. GRDB (SQLite) is the only conformer today, but it is unsupported
/// on Windows and unofficial on Linux; swapping it out later only requires a new
/// conformer here — call sites depend on this protocol (and the plain `SessionRecord`
/// type), never on GRDB directly.
public protocol SessionPersisting {
    // MARK: - Sessions

    func allSessions() throws -> [SessionRecord]
    func upsert(_ record: SessionRecord) throws
    func deleteSession(id: String) throws

    // MARK: - App state (e.g. last selection)

    func setSelectedSessionID(_ id: String?) throws
    func selectedSessionID() throws -> String?

    // MARK: - Generic key-value app state

    func setAppState(_ value: String?, forKey key: String) throws
    func appState(forKey key: String) throws -> String?
    /// `prefix` で始まるキーの全エントリ（キー→値）。
    func appStateEntries(prefix: String) throws -> [String: String]
}

extension SessionDatabase: SessionPersisting {}
