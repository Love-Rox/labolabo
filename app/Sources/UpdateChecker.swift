import Foundation
import Observation
import UserNotifications
import LaboLaboEngine

/// GitHub Releases を見て新しいバージョンの有無を調べる軽量アップデートチェッカ。
///
/// 署名/notarization・自動インストールは伴わない（無料配布路線の第一歩）。将来 Sparkle を
/// 入れたら「確認」→「自動ダウンロード/インストール」へ置き換え/昇格できる。
@MainActor
@Observable
final class UpdateChecker {
    static let shared = UpdateChecker()

    /// 「起動時に確認する」設定キー（既定 true）。
    static let autoCheckKey = "checkUpdatesOnLaunch"
    /// 「この version は通知済み」を覚える（起動ごとの通知連発を防ぐ）。
    private static let lastNotifiedKey = "lastNotifiedUpdateVersion"
    /// 直近の自動チェック時刻（起動連打で GitHub を叩きすぎないための throttle）。
    private static let lastAutoCheckKey = "lastAutoUpdateCheckAt"
    /// 自動チェックの最小間隔。
    private static let autoCheckMinInterval: TimeInterval = 6 * 60 * 60 // 6h

    enum State: Equatable {
        case idle
        case checking
        case upToDate
        case available(version: String, url: URL)
        case failed
    }

    private(set) var state: State = .idle

    /// 現在のアプリバージョン（CFBundleShortVersionString = MARKETING_VERSION）。読めなければ空。
    var currentVersion: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? ""
    }

    /// ビルド番号（CFBundleVersion）。
    var currentBuild: String {
        Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? ""
    }

    private init() {}

    /// GitHub の最新リリースと現在版を比較する。
    /// - Parameters:
    ///   - notifyIfAvailable: 新版発見時に通知も出す（同一 version は一度だけ・許可済みのみ）。
    ///   - throttle: 直近に自動チェック済みならスキップする（起動時チェック用）。手動は false。
    func check(notifyIfAvailable: Bool = false, throttle: Bool = false) {
        guard state != .checking else { return }
        let current = currentVersion
        // 版が読めないと「あらゆるリリースが新しい」と誤判定するので確認しない。
        guard !current.isEmpty else { state = .failed; return }
        if throttle,
           let last = UserDefaults.standard.object(forKey: Self.lastAutoCheckKey) as? Date,
           Date().timeIntervalSince(last) < Self.autoCheckMinInterval {
            return
        }
        state = .checking
        Task { [weak self] in
            guard let self else { return }
            let result = await Self.fetchLatest(current: current)
            self.state = result
            UserDefaults.standard.set(Date(), forKey: Self.lastAutoCheckKey)
            if notifyIfAvailable, case let .available(version, _) = result {
                await self.notifyIfNew(version: version)
            }
        }
    }

    /// 新版を「一度だけ・通知許可済みのとき」通知する。未許可（初回起動の許可待ち等）なら
    /// 投函せず記録もしないので、許可後の次回起動で改めて通知される。
    private func notifyIfNew(version: String) async {
        guard UserDefaults.standard.string(forKey: Self.lastNotifiedKey) != version else { return }
        guard await AgentNotifier.authorizationStatus() == .authorized else { return }
        AgentNotifier.postUpdateAvailable(version: version)
        UserDefaults.standard.set(version, forKey: Self.lastNotifiedKey)
    }

    /// ネットワーク取得＋判定（nonisolated: MainActor を占有しない）。
    private nonisolated static func fetchLatest(current: String) async -> State {
        var request = URLRequest(url: GitHubRepo.latestReleaseAPI)
        request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        request.timeoutInterval = 15
        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse, http.statusCode == 200,
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let tag = object["tag_name"] as? String else {
                return .failed
            }
            guard ReleaseVersion.isNewer(tag, than: current) else { return .upToDate }
            // 新版あり。html_url を使い、無い/壊れていてもリリース一覧にフォールバックする
            // （URL の失敗を「最新」と取り違えない）。
            let url = (object["html_url"] as? String).flatMap { URL(string: $0) } ?? GitHubRepo.releasesPage
            return .available(version: ReleaseVersion.normalize(tag), url: url)
        } catch {
            return .failed
        }
    }
}
