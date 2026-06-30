import SwiftUI

@main
struct LaboLaboApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
                .frame(minWidth: 1000, minHeight: 640)
        }
        // タイトルバーを隠し、上部の空きバーをなくして自前の 1 本バーに統合する。
        // サイドバー上部に "LaboLabo"＋開くボタン、詳細上部に自前の操作バーを置く。
        .windowStyle(.hiddenTitleBar)
    }
}
