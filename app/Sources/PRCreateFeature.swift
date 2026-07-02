import ComposableArchitecture
import Foundation
import AppKit
import LaboLaboEngine

/// PR 作成シートの TCA フィーチャ（段階導入の PoC）。
///
/// 状態遷移（読込→フォーム編集→push+作成→成否）を Reducer に集約し、engine への
/// 副作用は `PRCreateClient` 経由の Effect に閉じ込める。エンジン層（actor）は無改造。
@Reducer
struct PRCreateFeature {
    @ObservableState
    struct State: Equatable {
        /// 対象セッションの worktree と現在ブランチ（表示・読込に使う）。
        let worktree: URL
        let branch: String?

        var loading = true
        var branches: [String] = []
        var base = ""
        var title = ""
        var prBody = ""
        var draft = true
        var creating = false
        var errorText: String?
        /// 完了（作成成功 or キャンセル）。View が dismiss する。
        var finished = false

        var canCreate: Bool {
            !creating && !base.isEmpty && !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    enum Action: BindableAction, Equatable {
        case binding(BindingAction<State>)
        /// シート表示時（リポジトリ情報の読込）。
        case task
        case loaded(SessionStore.RepoInspect?)
        case createTapped
        case created(url: String)
        case createFailed(message: String)
        case cancelTapped
    }

    @Dependency(\.prCreateClient) var client

    var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding:
                return .none

            case .task:
                let worktree = state.worktree
                return .run { send in
                    await send(.loaded(client.inspect(worktree)))
                }

            case let .loaded(inspect):
                state.loading = false
                let current = inspect?.current ?? state.branch
                state.branches = (inspect?.branches ?? []).filter { $0 != current }
                // 運用に合わせて dev を優先、無ければ main、どちらも無ければ先頭。
                state.base = ["dev", "main"].first(where: { state.branches.contains($0) })
                    ?? state.branches.first ?? ""
                if state.title.isEmpty { state.title = inspect?.lastSubject ?? current ?? "" }
                return .none

            case .createTapped:
                guard state.canCreate else { return .none }
                state.creating = true
                state.errorText = nil
                let (base, title, body, draft) = (state.base, state.title, state.prBody, state.draft)
                return .run { send in
                    do {
                        let url = try await client.create(base, title, body, draft)
                        await send(.created(url: url))
                    } catch {
                        await send(.createFailed(message: PRCreateFeature.message(for: error)))
                    }
                }

            case let .created(url):
                state.finished = true
                return .run { _ in await client.open(url) }

            case let .createFailed(message):
                state.creating = false
                state.errorText = message
                return .none

            case .cancelTapped:
                state.finished = true
                return .none
            }
        }
    }

    /// エラーを UI 用の文言に。GitCommandError の stderr が空なら localizedDescription に落とす。
    static func message(for error: Error) -> String {
        if let git = error as? GitCommandError {
            let stderr = git.stderr.trimmingCharacters(in: .whitespacesAndNewlines)
            if !stderr.isEmpty { return stderr }
        }
        let description = error.localizedDescription
        return description.isEmpty ? "不明なエラーが発生しました。" : description
    }
}

/// フィーチャの副作用（engine 呼び出し）を注入するクライアント。セッション束縛の
/// クロージャを呼び出し側（ContentView）で組み立てて渡す。`@MainActor` 隔離により
/// `@MainActor` な SessionStore を安全に捕捉できる。
struct PRCreateClient {
    var inspect: @MainActor @Sendable (_ worktree: URL) async -> SessionStore.RepoInspect?
    var create: @MainActor @Sendable (_ base: String, _ title: String, _ body: String, _ draft: Bool) async throws -> String
    var open: @MainActor @Sendable (_ url: String) -> Void
}

extension PRCreateClient: DependencyKey {
    /// 既定は no-op（実体は呼び出し側で withDependencies で差し替える）。
    static let liveValue = PRCreateClient(
        inspect: { _ in nil },
        create: { _, _, _, _ in "" },
        open: { _ in }
    )
    static let testValue = liveValue
}

extension DependencyValues {
    var prCreateClient: PRCreateClient {
        get { self[PRCreateClient.self] }
        set { self[PRCreateClient.self] = newValue }
    }
}
