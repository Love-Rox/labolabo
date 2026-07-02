import Foundation

/// 外部 CLI（git / gh / claude …）の絶対パスを解決する唯一の窓口。
///
/// GUI アプリの PATH は launchd 由来で貧弱（`/usr/bin:/bin:…`）なため、
/// (1) 定番の固定候補、(2) プロセスの PATH、(3) ログインシェルの PATH
/// （`$SHELL -l -c 'command -v <name>'`）の順に探す。ログイン解決は端末で
/// コマンドを走らせたときの PATH に近く、doctor の判定と実際の起動可否を一致させる。
///
/// 判定を 1 か所に集約することで、doctor が「見つかる」と言ったツールを
/// エンジン側が実行できない（またはその逆）という UI と実体の食い違いを防ぐ。
public enum ToolLocator {

    /// 固定候補ディレクトリ（Homebrew / Nix / mise shims / ~/.local など）の和集合。
    private static func fixedCandidates(_ name: String) -> [String] {
        let home = NSHomeDirectory()
        return [
            "/opt/homebrew/bin/\(name)",
            "/usr/local/bin/\(name)",
            "/usr/bin/\(name)",
            "/run/current-system/sw/bin/\(name)", // Nix
            "\(home)/.local/bin/\(name)",
            "\(home)/.claude/local/\(name)",
            "\(home)/.local/share/mise/shims/\(name)",
        ]
    }

    /// `name` の絶対パスを返す（見つからなければ `nil`）。
    public static func locate(_ name: String) -> URL? {
        let fm = FileManager.default

        for path in fixedCandidates(name) where fm.isExecutableFile(atPath: path) {
            return URL(fileURLWithPath: path)
        }

        if let pathEnv = ProcessInfo.processInfo.environment["PATH"] {
            for dir in pathEnv.split(separator: ":") {
                let path = "\(dir)/\(name)"
                if fm.isExecutableFile(atPath: path) { return URL(fileURLWithPath: path) }
            }
        }

        // 最終手段: ログインシェルの PATH で解決（端末での起動 PATH に近い）。
        if let path = locateViaLoginShell(name), fm.isExecutableFile(atPath: path) {
            return URL(fileURLWithPath: path)
        }
        return nil
    }

    /// `$SHELL -l -c 'command -v <name>'`。`.zprofile` 等の出力が前置されうるので
    /// 絶対パス（`/` 始まり）の**最終行**を採用する。timeout でハングを防ぐ。
    private static func locateViaLoginShell(_ name: String) -> String? {
        // name は呼び出し側の定数（git/gh/claude）だが、念のため単純な語だけ許可する。
        guard name.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" }) else { return nil }
        let shell = ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh"
        guard let out = ProcessRunner.runSync(
            executable: URL(fileURLWithPath: shell),
            arguments: ["-l", "-c", "command -v \(name) 2>/dev/null"],
            timeout: 5
        ), out.status == 0 else { return nil }
        return out.stdout
            .split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .last(where: { $0.hasPrefix("/") })
    }
}
