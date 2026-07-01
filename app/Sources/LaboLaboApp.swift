import SwiftUI

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

/// 起動完了時に Dock アイコンの外観追従を開始する（実行中のみ切替可能）。
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        AppIconController.shared.start()
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
