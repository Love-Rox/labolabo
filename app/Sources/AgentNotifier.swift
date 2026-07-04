import Foundation
import UserNotifications

/// エージェントの「入力待ち」などを macOS 通知で知らせる。設定でオフにできる。
@MainActor
enum AgentNotifier {
    /// 通知の有効/無効（@AppStorage と共有）。既定 ON。
    static let enabledKey = "notifyWaitingForInput"

    private static var requested = false

    /// 起動時に一度だけ通知許可を要求し、前面表示できるよう delegate を設定する。
    static func configure(delegate: UNUserNotificationCenterDelegate) {
        UNUserNotificationCenter.current().delegate = delegate
        guard !requested else { return }
        requested = true
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    static var isEnabled: Bool {
        (UserDefaults.standard.object(forKey: enabledKey) as? Bool) ?? true
    }

    /// 入力待ちに入ったセッションを通知（設定 ON のときのみ）。
    static func notifyWaiting(sessionName: String, branch: String?) {
        guard isEnabled else { return }
        let content = UNMutableNotificationContent()
        content.title = "\(sessionName) が入力待ち"
        if let branch, !branch.isEmpty { content.subtitle = branch }
        content.body = "エージェントが入力・許可を待っています。"
        content.sound = .default
        let request = UNNotificationRequest(
            identifier: UUID().uuidString, content: content, trigger: nil
        )
        UNUserNotificationCenter.current().add(request)
    }

    /// 新しいバージョンが利用可能なことを通知する（識別子は version 由来）。通知の投函は
    /// この enum に集約する（UpdateChecker からの直投函を避け、所有権を一本化）。
    static func postUpdateAvailable(version: String) {
        let content = UNMutableNotificationContent()
        content.title = "LaboLabo の新しいバージョン"
        content.body = "v\(version) が利用可能です（設定 > 一般 から確認）。"
        content.sound = .default
        let request = UNNotificationRequest(
            identifier: "labolabo.update.\(version)", content: content, trigger: nil
        )
        UNUserNotificationCenter.current().add(request)
    }

    /// 現在の通知許可状態（`.authorized` のときだけ投函したい場面で使う）。
    static func authorizationStatus() async -> UNAuthorizationStatus {
        await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
    }
}
