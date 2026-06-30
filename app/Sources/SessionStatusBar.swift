import SwiftUI
import AppKit
import LaboLaboEngine

// ウインドウ上部のツールバー（"LaboLabo" タイトルのあるバー）に並べる部品群。
// 旧 SessionStatusBar（独立した横帯）はツールバーへ集約したため廃止し、
// ブランチ/状態の表示・IDE で開く・時計を個別の小さな View として提供する。

/// 「今のステータス」: ブランチ + ahead/behind + dirty/clean を 1 行で。
struct GitStatusBadges: View {
    let status: GitStatus?
    let fallbackBranch: String?

    var body: some View {
        HStack(spacing: 8) {
            Label(branchLabel, systemImage: "arrow.triangle.branch")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(1)

            if let status {
                if status.ahead > 0 {
                    Label("\(status.ahead)", systemImage: "arrow.up")
                        .labelStyle(.titleAndIcon)
                        .foregroundStyle(.secondary)
                }
                if status.behind > 0 {
                    Label("\(status.behind)", systemImage: "arrow.down")
                        .labelStyle(.titleAndIcon)
                        .foregroundStyle(.secondary)
                }
                dirtyChip(isDirty: status.isDirty)
            } else {
                Text("読み込み中…").foregroundStyle(.tertiary)
            }
        }
        .font(.caption)
    }

    private var branchLabel: String {
        if let status, status.isDetached { return "detached" }
        return status?.branch ?? fallbackBranch ?? "—"
    }

    private func dirtyChip(isDirty: Bool) -> some View {
        Text(isDirty ? "変更あり" : "クリーン")
            .font(.caption.weight(.medium))
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .background(Capsule().fill((isDirty ? Color.orange : Color.secondary).opacity(0.18)))
            .foregroundStyle(isDirty ? Color.orange : Color.secondary)
    }
}

/// 「IDE で開く」メニュー（ピル）。インストール済みエディタのみ表示。
struct IDEOpenMenu: View {
    let worktree: URL

    var body: some View {
        Menu {
            ForEach(installedEditors) { editor in
                Button { open(in: editor) } label: {
                    Label(editor.name, systemImage: "chevron.left.forwardslash.chevron.right")
                }
            }
            if !installedEditors.isEmpty { Divider() }
            Button {
                NSWorkspace.shared.activateFileViewerSelecting([worktree])
            } label: {
                Label("Finder で表示", systemImage: "folder")
            }
        } label: {
            HStack(spacing: 6) {
                Image(systemName: "arrow.up.forward.app")
                Text("IDE で開く")
                Image(systemName: "chevron.down")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            .font(.callout.weight(.medium))
            .pillFrame(prominent: true)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("worktree を任意の IDE / Finder で開く")
    }

    private func open(in editor: Editor) {
        NSWorkspace.shared.open(
            [worktree],
            withApplicationAt: editor.appURL,
            configuration: NSWorkspace.OpenConfiguration(),
            completionHandler: nil
        )
    }

    private var installedEditors: [Editor] {
        Editor.candidates.compactMap { candidate in
            guard let appURL = NSWorkspace.shared
                .urlForApplication(withBundleIdentifier: candidate.bundleID) else { return nil }
            return Editor(name: candidate.name, bundleID: candidate.bundleID, appURL: appURL)
        }
    }
}

/// 現在時刻の時計（ピル）。
struct SessionClock: View {
    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            HStack(spacing: 6) {
                Image(systemName: "clock").foregroundStyle(.secondary)
                Text(context.date, format: .dateTime.hour().minute().second())
                    .monospacedDigit()
            }
            .font(.system(.callout, design: .monospaced).weight(.medium))
            .pillFrame()
        }
        .fixedSize()
        .help("現在時刻")
    }
}

/// 「IDE で開く」メニューに出すエディタ 1 つ分。
private struct Editor: Identifiable {
    let name: String
    let bundleID: String
    let appURL: URL

    var id: String { bundleID }

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

// MARK: - 円形アイコンボタン

/// アイコン 1 つを丸い枠に収めるツールバー用ボタンスタイル。無効時は淡色、押下で軽く縮む。
struct CircleIconButtonStyle: ButtonStyle {
    var tint: Color?
    var diameter: CGFloat = 30

    func makeBody(configuration: Configuration) -> some View {
        IconBody(configuration: configuration, tint: tint, diameter: diameter)
    }

    private struct IconBody: View {
        let configuration: Configuration
        let tint: Color?
        let diameter: CGFloat
        @Environment(\.isEnabled) private var isEnabled

        var body: some View {
            configuration.label
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(tint ?? .primary)
                .frame(width: diameter, height: diameter)
                .background(
                    Circle().fill(
                        configuration.isPressed
                            ? Color.primary.opacity(0.14)
                            : Color(nsColor: .controlBackgroundColor)
                    )
                )
                .overlay(
                    Circle().strokeBorder(
                        (tint ?? Color.primary).opacity(0.18),
                        lineWidth: 1
                    )
                )
                .opacity(isEnabled ? 1 : 0.4)
                .scaleEffect(configuration.isPressed ? 0.92 : 1)
                .contentShape(Circle())
        }
    }
}

// MARK: - ピル型の枠

private extension View {
    /// Supacode 風の、少し大きめでピル型（角丸全周）の枠に収める。
    func pillFrame(prominent: Bool = false) -> some View {
        padding(.horizontal, 14)
            .padding(.vertical, 7)
            .background(
                Capsule(style: .continuous)
                    .fill(prominent
                        ? Color.accentColor.opacity(0.16)
                        : Color(nsColor: .controlBackgroundColor))
            )
            .overlay(
                Capsule(style: .continuous)
                    .strokeBorder(
                        prominent
                            ? Color.accentColor.opacity(0.45)
                            : Color.primary.opacity(0.12),
                        lineWidth: 1
                    )
            )
            .contentShape(Capsule(style: .continuous))
    }
}
