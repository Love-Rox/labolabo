import SwiftUI
import AppKit

/// ホストの `NSWindow` のサイズ・位置・スクリーンを UserDefaults に自前で保存/復元する。
///
/// SwiftUI 標準のウィンドウ復元や `setFrameAutosaveName` は、起動直後に SwiftUI が
/// ウィンドウをメインスクリーンへ再配置 → その move が保存を誘発して外部モニタの位置を
/// 上書きしてしまい、複数モニタで位置が記憶されない。これを避けるため autosave は使わず、
/// (1) 起動時の SwiftUI 再配置に負けないよう少し遅延して復元し、
/// (2) 起動が落ち着いてからのみ保存を有効化する。
struct WindowAccessor: NSViewRepresentable {
    let defaultsKey: String

    func makeNSView(context: Context) -> NSView { FrameTracker(defaultsKey: defaultsKey) }
    func updateNSView(_ nsView: NSView, context: Context) {}
}

private final class FrameTracker: NSView {
    private let defaultsKey: String
    private var configured = false
    private var savingEnabled = false

    init(defaultsKey: String) {
        self.defaultsKey = defaultsKey
        super.init(frame: .zero)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard let window, !configured else { return }
        configured = true
        // システム/SwiftUI の状態復元を切り、位置の記憶を自前管理に一本化する。
        window.isRestorable = false

        restoreFrame()
        // SwiftUI の初期配置後に確実に復元位置へ寄せる（メインへ戻されるのを上書き）。
        perform(#selector(restoreFrame), with: nil, afterDelay: 0.2)
        // 起動時の再配置を保存しないよう、落ち着いてから保存を有効化＋監視開始。
        perform(#selector(enableSaving), with: nil, afterDelay: 0.6)
    }

    @objc private func restoreFrame() {
        guard let window, let string = UserDefaults.standard.string(forKey: defaultsKey) else { return }
        let rect = NSRectFromString(string)
        guard rect.width > 200, rect.height > 150 else { return }
        window.setFrame(rect, display: true)
    }

    @objc private func enableSaving() {
        guard let window else { return }
        savingEnabled = true
        NotificationCenter.default.addObserver(
            self, selector: #selector(saveFrame), name: NSWindow.didMoveNotification, object: window
        )
        NotificationCenter.default.addObserver(
            self, selector: #selector(saveFrame), name: NSWindow.didResizeNotification, object: window
        )
    }

    @objc private func saveFrame() {
        guard savingEnabled, let window else { return }
        UserDefaults.standard.set(NSStringFromRect(window.frame), forKey: defaultsKey)
    }

    deinit { NotificationCenter.default.removeObserver(self) }
}
