import SwiftUI
import AppKit
import LaboLaboEngine

/// セッション 1 つ分のヘッダー（Supacode 風ツールバー）。
///
/// 左からセッション名・ブランチ・ライブな Git ステータス、右側へ「IDE で開く」メニュー、
/// 現在時刻の時計、そして閉じるボタンを並べる。`status` は `WorkPane` 側で監視している
/// ライブな `GitStatus`（FSEvents 更新）を流し込む想定で、`nil` の間は「読み込み中…」を出す。
struct SessionStatusBar: View {
    let session: RepoSession
    let status: GitStatus?
    var onClose: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            sessionInfo
            Spacer()
            openInIDEMenu
            clock
            closeButton
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    // MARK: - 左: セッション名 + ブランチ + ステータス

    private var sessionInfo: some View {
        HStack(spacing: 10) {
            Text(session.name)
                .font(.headline)
                .lineLimit(1)

            Label(branchLabel, systemImage: "arrow.triangle.branch")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(1)

            statusArea
        }
    }

    /// detached の場合はブランチ名の代わりに「detached」を出す。
    private var branchLabel: String {
        if let status, status.isDetached { return "detached" }
        return status?.branch ?? session.branch ?? "—"
    }

    /// 「今のステータス」: ahead/behind のミニラベルと dirty/clean チップ。
    @ViewBuilder
    private var statusArea: some View {
        if let status {
            HStack(spacing: 8) {
                if status.ahead > 0 {
                    Label("\(status.ahead)", systemImage: "arrow.up")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .labelStyle(.titleAndIcon)
                }
                if status.behind > 0 {
                    Label("\(status.behind)", systemImage: "arrow.down")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .labelStyle(.titleAndIcon)
                }
                dirtyChip(isDirty: status.isDirty)
            }
        } else {
            Text("読み込み中…")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
    }

    private func dirtyChip(isDirty: Bool) -> some View {
        Text(isDirty ? "変更あり" : "クリーン")
            .font(.caption.weight(.medium))
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .background(
                Capsule().fill(
                    (isDirty ? Color.orange : Color.secondary).opacity(0.18)
                )
            )
            .foregroundStyle(isDirty ? Color.orange : Color.secondary)
    }

    // MARK: - 中央右: 「IDE で開く」メニュー

    private var openInIDEMenu: some View {
        Menu {
            ForEach(installedEditors) { editor in
                Button {
                    open(in: editor)
                } label: {
                    Label(editor.name, systemImage: "chevron.left.forwardslash.chevron.right")
                }
            }

            if !installedEditors.isEmpty {
                Divider()
            }

            Button {
                NSWorkspace.shared.activateFileViewerSelecting([session.worktreePath])
            } label: {
                Label("Finder で表示", systemImage: "folder")
            }
        } label: {
            Label("IDE で開く", systemImage: "arrow.up.forward.app")
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .help("worktree を任意の IDE / Finder で開く")
    }

    /// worktree フォルダを指定エディタで開く。
    private func open(in editor: Editor) {
        NSWorkspace.shared.open(
            [session.worktreePath],
            withApplicationAt: editor.appURL,
            configuration: NSWorkspace.OpenConfiguration(),
            completionHandler: nil
        )
    }

    /// インストール済みの主要エディタだけを `body` から計算で組み立てる。
    private var installedEditors: [Editor] {
        Editor.candidates.compactMap { candidate in
            guard let appURL = NSWorkspace.shared
                .urlForApplication(withBundleIdentifier: candidate.bundleID) else { return nil }
            return Editor(name: candidate.name, bundleID: candidate.bundleID, appURL: appURL)
        }
    }

    // MARK: - 右: 現在時刻の時計

    private var clock: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            Text(context.date, format: .dateTime.hour().minute().second())
                .font(.system(.callout, design: .monospaced))
                .foregroundStyle(.secondary)
                .monospacedDigit()
        }
        .help("現在時刻")
    }

    // MARK: - 右端: 閉じるボタン

    private var closeButton: some View {
        Button(role: .destructive) {
            onClose()
        } label: {
            Image(systemName: "xmark.circle.fill")
        }
        .buttonStyle(.borderless)
        .help("セッションを閉じる")
    }
}

/// 「IDE で開く」メニューに出すエディタ 1 つ分。
private struct Editor: Identifiable {
    let name: String
    let bundleID: String
    /// インストール済みと判明したアプリの URL。
    let appURL: URL

    var id: String { bundleID }

    /// 表示名とバンドル ID の候補。`appURL` はインストール判定後に埋める。
    struct Candidate {
        let name: String
        let bundleID: String
    }

    /// 主要エディタの候補（インストール済みのものだけを実際にメニューへ出す）。
    static let candidates: [Candidate] = [
        Candidate(name: "Visual Studio Code", bundleID: "com.microsoft.VSCode"),
        Candidate(name: "Cursor", bundleID: "com.todesktop.230313mzl4w4u92"),
        Candidate(name: "Zed", bundleID: "dev.zed.Zed"),
        Candidate(name: "Sublime Text", bundleID: "com.sublimetext.4"),
        Candidate(name: "JetBrains Fleet", bundleID: "Fleet"),
        Candidate(name: "Xcode", bundleID: "com.apple.dt.Xcode")
    ]
}
