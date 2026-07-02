import Foundation
import Observation

/// 外部ツール（git / gh / claude）の存在とバージョンを起動時に検査する。
/// 見つからないツールに依存する機能は UI 側で無効化し、理由を明示する（doctor）。
///
/// GUI アプリの PATH は launchd 由来で乏しい（/usr/bin:/bin:…）ため、Homebrew や
/// mise shims などの定番パスを明示的に探す。
@MainActor
@Observable
final class ToolDoctor {
    struct Tool {
        var found = false
        var version: String?
        var path: String?
    }

    static let shared = ToolDoctor()

    private(set) var git = Tool()
    private(set) var gh = Tool()
    private(set) var claude = Tool()
    private(set) var checking = false

    private init() {}

    /// 3 ツールをバックグラウンドで検査して結果を反映する。起動時と「再検査」で呼ぶ。
    func check() {
        guard !checking else { return }
        checking = true
        Task.detached(priority: .utility) {
            let git = Self.probe(name: "git")
            let gh = Self.probe(name: "gh")
            let claude = Self.probe(name: "claude")
            await MainActor.run {
                let doctor = ToolDoctor.shared
                doctor.git = git
                doctor.gh = gh
                doctor.claude = claude
                doctor.checking = false
            }
        }
    }

    // MARK: - probe（nonisolated: バックグラウンドで実行）

    private nonisolated static func probe(name: String) -> Tool {
        guard let path = locate(name: name) else { return Tool() }
        return Tool(found: true, version: runVersion(path: path), path: path)
    }

    private nonisolated static func locate(name: String) -> String? {
        let fm = FileManager.default
        let home = NSHomeDirectory()
        let candidates = [
            "/opt/homebrew/bin/\(name)",
            "/usr/local/bin/\(name)",
            "/usr/bin/\(name)",
            "\(home)/.local/bin/\(name)",
            "\(home)/.claude/local/\(name)",
            "\(home)/.local/share/mise/shims/\(name)",
        ]
        for candidate in candidates where fm.isExecutableFile(atPath: candidate) {
            return candidate
        }
        if let pathEnv = ProcessInfo.processInfo.environment["PATH"] {
            for dir in pathEnv.split(separator: ":") {
                let path = "\(dir)/\(name)"
                if fm.isExecutableFile(atPath: path) { return path }
            }
        }
        return nil
    }

    /// `<tool> --version` の 1 行目（失敗時 nil。存在はしているので found は維持）。
    private nonisolated static func runVersion(path: String) -> String? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: path)
        process.arguments = ["--version"]
        let out = Pipe()
        process.standardOutput = out
        process.standardError = Pipe()
        do { try process.run() } catch { return nil }
        let data = out.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { return nil }
        return String(decoding: data, as: UTF8.self)
            .split(separator: "\n").first
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
    }
}
