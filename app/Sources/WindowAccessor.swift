import SwiftUI
import AppKit

/// ホストの `NSWindow` を取得し、frame の autosave（サイズ・位置・スクリーン）を有効にする。
///
/// SwiftUI 標準の状態復元は複数モニタ環境で位置がずれることがあるため、AppKit の
/// `setFrameAutosaveName` に委ねる。autosave 名ごとに UserDefaults へ frame を保存し、
/// 次回起動時に（保存時と同じスクリーン構成なら）その位置・サイズへ復元する。
struct WindowAccessor: NSViewRepresentable {
    let autosaveName: String

    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        apply(from: view)
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        apply(from: nsView)
    }

    private func apply(from view: NSView) {
        // window は次のランループで確定するので遅延して取得する。
        DispatchQueue.main.async {
            guard let window = view.window,
                  window.frameAutosaveName != autosaveName else { return }
            // autosave を有効化し、保存済み frame があれば復元する。
            window.setFrameAutosaveName(autosaveName)
            window.setFrameUsingName(autosaveName)
        }
    }
}
