import XCTest

/// アプリ起動とメニュー操作のスモークテスト（クラッシュ検知）。
///
/// 過去に「表示(View)メニューを開くと無限再帰でクラッシュ」という不具合が出荷された。
/// XCUITest でアプリを外部から起動し、各メニューを開いても落ちないことを確認する。
/// XCUITest はアプリを別プロセスとして駆動するため `@testable import` は使わない。
///
/// 方針:
/// - 待機は 1〜2 秒と短く保つ。
/// - 子メニュー項目のラベル（ローカライズ依存）には一切依存しない。
///   検証するのは「アプリが生存し続けている」ことのみ。
// XCUIApplication / XCUIElement は現行 SDK で MainActor 隔離のため、クラスごと隔離する。
@MainActor
final class AppLaunchSmokeTests: XCTestCase {

    /// アプリが前面で起動し、ウィンドウが 1 つ以上生成されること。
    func testLaunches() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launch()

        XCTAssertEqual(app.state, .runningForeground, "アプリが前面で起動していない")
        // ウィンドウ生成にわずかな猶予を与える。
        XCTAssertTrue(
            app.windows.firstMatch.waitForExistence(timeout: 3),
            "ウィンドウが 1 つも生成されていない"
        )
        XCTAssertGreaterThan(app.windows.count, 0, "ウィンドウ数が 0")
    }

    /// メニューバーの各トップメニューを開いても（特に「表示」）クラッシュしないこと。
    func testOpensEachMenuWithoutCrashing() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launch()

        XCTAssertEqual(app.state, .runningForeground, "起動直後にアプリが前面でない")

        let titles = ["LaboLabo", "File", "Edit", "View", "Window", "Help"]
        for title in titles {
            openThenDismissMenu(named: title, in: app)
        }

        // 全メニューを開閉した後もアプリが生存していること＝クラッシュしていない。
        XCTAssertEqual(
            app.state,
            .runningForeground,
            "メニュー開閉のいずれかでアプリがクラッシュ/終了した"
        )
    }

    /// 設定画面（⌘,）を開いても落ちないこと。
    func testOpensSettings() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launch()

        XCTAssertEqual(app.state, .runningForeground, "起動直後にアプリが前面でない")

        // 設定ウィンドウを開く。
        app.typeKey(",", modifierFlags: .command)
        _ = app.windows.firstMatch.waitForExistence(timeout: 2)

        XCTAssertEqual(
            app.state,
            .runningForeground,
            "設定画面を開いた直後にアプリがクラッシュ/終了した"
        )

        // 開いていれば設定ウィンドウを閉じる（後片付け）。⌘W が無害な no-op でも問題ない。
        app.typeKey("w", modifierFlags: .command)
        XCTAssertEqual(
            app.state,
            .runningForeground,
            "設定画面を閉じた後にアプリがクラッシュ/終了した"
        )
    }

    // MARK: - Helpers

    /// 指定タイトルのトップメニューを開き、少し待ってから Esc で閉じる。
    ///
    /// - メニューが存在しない環境（ローカライズ差・項目非表示）ではスキップし、失敗にはしない。
    /// - アプリメニューはプロセス名で解決できないことがあるため、"LaboLabo" が
    ///   見つからなければ先頭のメニューバー項目にフォールバックする。
    private func openThenDismissMenu(named title: String, in app: XCUIApplication) {
        let menuBar = app.menuBars
        var item = menuBar.menuBarItems[title]

        if !item.exists {
            if title == "LaboLabo" {
                // アプリメニューは先頭項目にフォールバック。
                let first = menuBar.menuBarItems.element(boundBy: 0)
                guard first.exists else { return }
                item = first
            } else {
                // 該当メニューが無い環境ではスキップ（失敗にしない）。
                return
            }
        }

        item.click()
        // メニュー展開の描画にわずかな猶予。ここで再帰クラッシュすれば以降で検知される。
        _ = app.menuItems.firstMatch.waitForExistence(timeout: 1)
        // 子メニュー項目には触れず、Esc で閉じるだけ。
        app.typeKey(XCUIKeyboardKey.escape, modifierFlags: [])
    }
}
