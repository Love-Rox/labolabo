import SwiftUI
import ComposableArchitecture
import LaboLaboEngine

/// 現在ブランチから Pull Request を作成するシート（TCA 版）。
/// 状態・副作用は `PRCreateFeature` に集約し、View は Store の描画に徹する。
struct PRCreateSheet: View {
    @Bindable var store: StoreOf<PRCreateFeature>
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Pull Request を作成")
                .font(.headline)
            HStack(spacing: 4) {
                Image(systemName: "arrow.triangle.branch").font(.caption2)
                Text(store.branch ?? "—")
                    .font(.caption.monospaced())
                Text("→ push してから PR を作成します")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(.bottom, 10)

            Form {
                if store.loading {
                    HStack { ProgressView().controlSize(.small); Text("リポジトリ情報を取得中…").foregroundStyle(.secondary) }
                } else {
                    Picker("ベースブランチ", selection: $store.base) {
                        ForEach(store.branches, id: \.self) { Text($0).tag($0) }
                    }
                    TextField("タイトル", text: $store.title)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("本文（任意・Closes #N で Issue に紐づけ）")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        TextEditor(text: $store.prBody)
                            .font(.body)
                            .frame(height: 110)
                            .overlay(
                                RoundedRectangle(cornerRadius: 6)
                                    .strokeBorder(.quaternary, lineWidth: 1)
                            )
                    }
                    Toggle("Draft として作成", isOn: $store.draft)
                }

                if let errorText = store.errorText {
                    Label(errorText, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                        .font(.caption)
                        .textSelection(.enabled)
                }
            }
            .formStyle(.grouped)

            HStack {
                Spacer()
                Button("キャンセル", role: .cancel) { store.send(.cancelTapped) }
                    .keyboardShortcut(.cancelAction)
                Button(store.creating ? "作成中…" : "push して作成") { store.send(.createTapped) }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!store.canCreate)
            }
            .padding(.top, 12)
        }
        .padding(20)
        .frame(width: 560)
        .task { store.send(.task) }
        .onChange(of: store.finished) { _, finished in
            if finished { dismiss() }
        }
    }
}
