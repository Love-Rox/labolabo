import Foundation
import Observation
import LaboLaboEngine

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
        guard let url = ToolLocator.locate(name) else { return Tool() }
        return Tool(found: true, version: runVersion(url: url), path: url.path)
    }

    /// `<tool> --version` の 1 行目（失敗時 nil。存在はしているので found は維持）。
    /// 両パイプ drain＋timeout の `ProcessRunner` を使い、ハングで `checking` が
    /// 固まらないようにする。
    private nonisolated static func runVersion(url: URL) -> String? {
        guard let out = ProcessRunner.runSync(executable: url, arguments: ["--version"], timeout: 5),
              out.status == 0 else { return nil }
        return out.stdout
            .split(separator: "\n").first
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
    }
}
