import Foundation
import GhosttyTerminal

/// Resolves which Ghostty configuration the embedded terminals should use.
/// If the user already has a Ghostty.app config on disk, we load it so the
/// embedded terminal matches their normal Ghostty experience (theme, font,
/// keybinds…). Otherwise we fall back to libghostty defaults.
enum GhosttyConfig {
    /// `.file(path)` for the first existing user config, else `.none`.
    static func userConfigSource() -> TerminalController.ConfigSource {
        if let path = userConfigPath() {
            return .file(path)
        }
        return .none
    }

    /// First existing Ghostty config path, searched in Ghostty's own priority order.
    static func userConfigPath() -> String? {
        let fm = FileManager.default
        var candidates: [String] = []

        let env = ProcessInfo.processInfo.environment
        if let xdg = env["XDG_CONFIG_HOME"], !xdg.isEmpty {
            candidates.append((xdg as NSString).appendingPathComponent("ghostty/config"))
        }
        let home = fm.homeDirectoryForCurrentUser
        candidates.append(home.appendingPathComponent(".config/ghostty/config").path)
        candidates.append(
            home.appendingPathComponent("Library/Application Support/com.mitchellh.ghostty/config").path
        )

        return candidates.first { fm.fileExists(atPath: $0) }
    }
}
