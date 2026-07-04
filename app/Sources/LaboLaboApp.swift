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
    private var quitMonitor: Any?

    func applicationDidFinishLaunching(_ notification: Notification) {
        AppIconController.shared.start()
        AgentNotifier.configure(delegate: self)
        ToolDoctor.shared.check() // git/gh/claude の存在検査（依存機能のゲートに使う）
        installQuitShortcut()
        // 起動時アップデート確認（既定 ON・未設定時も ON）。新版発見なら通知。
        // throttle: 起動連打で GitHub を叩きすぎないよう直近チェック済みならスキップ。
        if (UserDefaults.standard.object(forKey: UpdateChecker.autoCheckKey) as? Bool) ?? true {
            UpdateChecker.shared.check(notifyIfAvailable: true, throttle: true)
        }
    }

    /// ⌘Q を確実に効かせる。埋め込み libghostty 端末がフォーカス時に `cmd+q`（ghostty の
    /// キーバインド）を握ってメニューの Quit まで届かないため、ローカルイベントモニタで
    /// ⌘Q を端末より先に横取りしてアプリを終了する。
    private func installQuitShortcut() {
        quitMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            let mods = event.modifierFlags.intersection([.command, .shift, .option, .control])
            if mods == .command, event.charactersIgnoringModifiers?.lowercased() == "q" {
                NSApp.terminate(nil)
                return nil // 端末へ渡さず消費する
            }
            return event
        }
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
                // ウィンドウのサイズ・位置・スクリーンを記憶（複数モニタでも復元）。
                .background(WindowAccessor(autosaveName: "LaboLaboMainWindow"))
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
