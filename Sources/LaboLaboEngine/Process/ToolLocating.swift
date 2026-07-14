import Foundation

/// 外部 CLI ツール名 → 絶対パス解決の抽象化。
///
/// macOS 版は固定候補 + PATH + ログインシェルで解決する `ToolLocator` が実装する。
/// 将来 Windows 版を足すときは `where` コマンドベースの実装をこの protocol に
/// 差し込める（呼び出し側は `ToolLocating` にしか依存しない形にしていく）。
public protocol ToolLocating {
    /// `name` の絶対パスを返す（見つからなければ `nil`）。
    static func locate(_ name: String) -> URL?
}

extension ToolLocator: ToolLocating {}
