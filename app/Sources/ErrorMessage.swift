import Foundation
import LaboLaboEngine

extension Error {
    /// UI 表示用のエラー文言。`GitCommandError` は stderr（空/空白なら localizedDescription）を
    /// 優先する。空 stderr で空ラベルにならないよう、最終的に必ず非空の文言を返す。
    ///
    /// PR 作成・New Session・worktree 削除など、git/gh 失敗をユーザーに見せる箇所で共通利用する。
    var sessionUIMessage: String {
        if let git = self as? GitCommandError {
            let stderr = git.stderr.trimmingCharacters(in: .whitespacesAndNewlines)
            if !stderr.isEmpty { return stderr }
        }
        let description = localizedDescription
        return description.isEmpty ? "不明なエラーが発生しました。" : description
    }
}
