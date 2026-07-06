import Foundation
import GhosttyTerminal

/// Resolves which Ghostty configuration the embedded terminals should use.
/// If the user already has a Ghostty.app config on disk, we load it so the
/// embedded terminal matches their normal Ghostty experience (theme, font,
/// keybinds…). Otherwise we fall back to libghostty defaults.
enum GhosttyConfig {
    /// `.file(path)` for the first existing user config, else a generated default.
    static func userConfigSource() -> TerminalController.ConfigSource {
        if let path = userConfigPath() {
            return .file(path)
        }
        // ユーザー config が無いときだけ Web と同じ ink 背景に寄せる。
        // 既存のユーザー設定があればそちらを尊重し、この既定は使わない。
        if let path = generatedDefaultConfigPath() {
            return .file(path)
        }
        return .none
    }

    /// アプリ生成の既定 config を Application Support に書き出してパスを返す。
    /// 書き込みに失敗した場合は nil（従来どおり libghostty の既定にフォールバック）。
    private static func generatedDefaultConfigPath() -> String? {
        let fm = FileManager.default
        let dir = fm.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/LaboLabo")
        let file = dir.appendingPathComponent("ghostty-default-config")
        let contents = "background = 0b0b0e\nforeground = e8e8ec\n"
        do {
            try fm.createDirectory(at: dir, withIntermediateDirectories: true)
            try contents.write(to: file, atomically: true, encoding: .utf8)
            return file.path
        } catch {
            return nil
        }
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
