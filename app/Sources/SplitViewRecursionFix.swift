import AppKit
import ObjectiveC.runtime

/// macOS 26 (Tahoe) の AppKit 不具合対策 —— メニューを開くとクラッシュする問題。
///
/// メニューを開くと AppKit は各項目の有効/無効を検証するため responder chain を辿る
/// (`-[NSApplication targetForAction:to:from:]` →
///  `_objectFromResponderChainWhichRespondsToAction`)。その途中で NavigationSplitView の
/// サイドバー用 `NSSplitView` に `respondsToSelector:` を送るが、macOS 26.5 の
/// `-[NSSplitView(NSSplitViewSidebar) respondsToSelector:]` は弱参照経由で自分自身へ
/// `respondsToSelector:` を転送し、無限再帰してスタックを溢れさせクラッシュする
/// （Help など任意のメニューを開くと落ちる。EXC_BAD_ACCESS / stack guard）。
///
/// 対策として `-[NSSplitView respondsToSelector:]` を swizzle し、**同一オブジェクトへの
/// 再入（循環）を検出したら本来の（バグった）実装を再帰させず、クラスレベルの素の応答可否**を
/// 返して循環を断ち切る。非循環時は元実装をそのまま呼ぶので通常の挙動は変わらない。
/// `class_respondsToSelector` は `respondsToSelector:` メッセージを介さずメソッドリストを
/// 直接見るため、ここから再帰は起きない。
///
/// 注: `CommandGroup(replacing: .sidebar) {}`（#52）はサイドバー開閉の項目を消すだけで、
/// 再帰は他のメニュー項目の検証でも起きるため、この swizzle が本質的な修正。
enum SplitViewRecursionFix {
    /// スレッドごとに「今 respondsToSelector 実行中の NSSplitView」を記録して再入を検出する。
    private final class ReentryGuard { var ids = Set<UInt>() }
    private static let tlsKey = "labolabo.splitview.respondsToSelector.guard"
    // 起動時に main スレッドで一度だけ設定するので unsafe で問題ない。
    nonisolated(unsafe) private static var installed = false

    /// アプリの UI 構築前に一度だけ呼ぶ（`AppEntry.main()` で実行）。
    static func install() {
        guard !installed else { return }
        installed = true

        let selector = #selector(NSObject.responds(to:))
        guard let method = class_getInstanceMethod(NSSplitView.self, selector) else { return }

        typealias OrigFn = @convention(c) (AnyObject, Selector, Selector?) -> ObjCBool
        let original = unsafeBitCast(method_getImplementation(method), to: OrigFn.self)

        let block: @convention(block) (AnyObject, Selector?) -> ObjCBool = { object, querySelector in
            let dict = Thread.current.threadDictionary
            let guardBox: ReentryGuard
            if let existing = dict[tlsKey] as? ReentryGuard {
                guardBox = existing
            } else {
                guardBox = ReentryGuard()
                dict[tlsKey] = guardBox
            }

            let id = UInt(bitPattern: Unmanaged.passUnretained(object).toOpaque())
            if guardBox.ids.contains(id) {
                // 再入（循環）検出: 元実装を再帰させず、素のクラス応答可否で断ち切る。
                guard let s = querySelector else { return false }
                return ObjCBool(class_respondsToSelector(object_getClass(object), s))
            }

            guardBox.ids.insert(id)
            defer { guardBox.ids.remove(id) }
            return original(object, selector, querySelector)
        }

        method_setImplementation(method, imp_implementationWithBlock(block))
    }
}
