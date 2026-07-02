import SwiftUI
import UserNotifications

/// 実行エントリ。`--hook <socket>` 付きで起動された場合は GUI を立ち上げず、
/// Claude hook の stdin を当該ソケットへ転送して即終了するフォワーダとして動く
/// （アプリ自身を同梱フォワーダとして使う＝別バイナリ不要）。それ以外は通常の GUI。
@main
enum AppEntry {
    static func main() {
        let args = CommandLine.arguments
        if let index = args.firstIndex(of: "--hook"), index + 1 < args.count {
            HookForwarder.forward(socketPath: args[index + 1])
            return
        }
        LaboLaboApp.main()
    }
}

/// 起動完了時に Dock アイコンの外観追従と、入力待ち通知の準備を行う。
final class AppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        AppIconController.shared.start()
        AgentNotifier.configure(delegate: self)
        ToolDoctor.shared.check() // git/gh/claude の存在検査（依存機能のゲートに使う）
    }

    /// アプリ前面時も通知を表示する（別セッションで作業中に入力待ちを知らせるため）。
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .sound])
    }
}

struct LaboLaboApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        WindowGroup {
            ContentView()
                .frame(minWidth: 1000, minHeight: 640)
        }
        // タイトルバーを隠し、上部の空きバーをなくして自前の 1 本バーに統合する。
        // サイドバー上部に "LaboLabo"＋開くボタン、詳細上部に自前の操作バーを置く。
        .windowStyle(.hiddenTitleBar)

        // 設定画面（⌘,）。アプリアイコンの表示モードなど。
        Settings {
            SettingsView()
        }
    }
}
