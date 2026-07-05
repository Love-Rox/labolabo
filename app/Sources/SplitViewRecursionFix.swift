import AppKit

/// macOS 26 (Tahoe) の AppKit 不具合対策 —— メニューを開くとクラッシュ/フリーズする問題。
///
/// メニューを開くと AppKit は各項目の有効/無効を自動検証する（`_sendMenuEnableItems`）。
/// その過程で、responder chain 上の NavigationSplitView サイドバー `NSSplitView` の
/// `NSSplitViewSidebar` カテゴリ（`respondsToSelector:` / `validateUserInterfaceItem:`）が
/// 弱参照経由で自己参照し、**無限再帰（EXC_BAD_ACCESS でクラッシュ）**または
/// **長大ループ（ビーチボールでフリーズ）**になる。項目種別に依らないので
/// `CommandGroup(replacing: .sidebar)`（#52）では防げず、`respondsToSelector:` だけを
/// swizzle しても `validateUserInterfaceItem:` 側でループするだけ（＝クラッシュがハングに化ける）。
///
/// 対策は**メニュー項目の自動検証そのものを無効化**すること（`NSMenu.autoenablesItems = false`）。
/// これで項目検証の responder chain 走査が走らなくなり、バグった経路に入らない。代償として
/// 一部の標準項目（Undo/Cut など）が状況に依らず有効表示になるが、ファーストレスポンダが
/// 処理しなければ押しても無害。本アプリはカスタムメニューがほぼ無いので影響は軽微。
///
/// SwiftUI がメニューを組み直しても効くよう、起動後に一度適用し、以降は**メニュー追跡が
/// 始まるたび**（`NSMenuDidBeginTracking`／個別メニューが開く前）にメインメニュー全体へ再適用する。
enum SplitViewRecursionFix {
    nonisolated(unsafe) private static var installed = false

    /// `AppEntry.main()` から UI 構築前に一度だけ呼ぶ（NSApp 未生成のうちに監視だけ仕込む）。
    static func install() {
        guard !installed else { return }
        installed = true

        // メニュー追跡が始まるたびに、個別メニューの検証が走る前へ間に合うよう全体へ再適用。
        NotificationCenter.default.addObserver(
            forName: NSMenu.didBeginTrackingNotification, object: nil, queue: .main
        ) { _ in
            MainActor.assumeIsolated { applyToMainMenu() }
        }
        // SwiftUI がメインメニューを構築した後に初回適用（起動直後は mainMenu が nil）。
        DispatchQueue.main.async {
            MainActor.assumeIsolated { applyToMainMenu() }
        }
    }

    @MainActor
    private static func applyToMainMenu() {
        guard let main = NSApp.mainMenu else { return }
        disableAutoenable(main)
    }

    @MainActor
    private static func disableAutoenable(_ menu: NSMenu) {
        menu.autoenablesItems = false
        for item in menu.items {
            if let submenu = item.submenu { disableAutoenable(submenu) }
        }
    }
}
