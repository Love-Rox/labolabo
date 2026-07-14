import Foundation

/// Resolves the root directory this app stores its persisted data under.
///
/// Only macOS is implemented today. Future platforms branch here instead of at each
/// call site: Windows would resolve `%APPDATA%\LaboLabo`, Linux would resolve
/// `$XDG_DATA_HOME/LaboLabo` (falling back to `~/.local/share/LaboLabo`).
public enum AppDataDirectory {
    /// `~/Library/Application Support/LaboLabo`
    public static func url() -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("LaboLabo")
    }
}
