import XCTest
import SwiftUI
import LaboLaboEngine
@testable import LaboLabo

/// UI に出す表示ラベル/パレットの単一情報源（`FileListMode` / `FileViewMode` /
/// `AppIconMode` / `AgentStatus.label` / `RepoPalette`）を横断的に検証する。
/// ラベルが欠落・重複していないこと、パレットの参照 API が安定していることを確認する。
final class DisplayLabelsTests: XCTestCase {

    // MARK: - FileListMode

    /// 3 モード（変更ツリー / 全体ツリー / 更新順）が列挙され、`id` は rawValue と一致する。
    func testFileListModeCasesAndID() {
        let all = FileListMode.allCases
        XCTAssertEqual(all, [.changedTree, .fullTree, .recent])
        for mode in all {
            XCTAssertEqual(mode.id, mode.rawValue)
        }
    }

    /// 各モードのラベルは非空、かつ全モードで重複しない（プルダウンの表示に使う）。
    func testFileListModeLabelsAreNonEmptyAndDistinct() {
        let labels = FileListMode.allCases.map(\.label)
        XCTAssertEqual(labels.count, FileListMode.allCases.count)
        for label in labels {
            XCTAssertFalse(label.isEmpty, "FileListMode.label が空")
        }
        XCTAssertEqual(Set(labels).count, labels.count, "FileListMode のラベルに重複がある")
    }

    // MARK: - FileViewMode

    /// Diff / 全文 の 2 モードが列挙され、`id` は rawValue と一致し重複しない。
    func testFileViewModeCasesAndIDs() {
        let all = FileViewMode.allCases
        XCTAssertFalse(all.isEmpty)
        XCTAssertEqual(all, [.diff, .whole])
        let ids = all.map(\.id)
        for mode in all {
            XCTAssertEqual(mode.id, mode.rawValue)
        }
        XCTAssertEqual(Set(ids).count, ids.count, "FileViewMode の id に重複がある")
    }

    // MARK: - AppIconMode

    /// Dock アイコンの 3 モードのラベルは非空かつ重複せず、`id` は rawValue と一致する。
    func testAppIconModeLabelsAreNonEmptyAndDistinct() {
        let all = AppIconMode.allCases
        XCTAssertEqual(all, [.auto, .dark, .light])
        let labels = all.map(\.label)
        for (mode, label) in zip(all, labels) {
            XCTAssertFalse(label.isEmpty, "AppIconMode.label が空: \(mode)")
            XCTAssertEqual(mode.id, mode.rawValue)
        }
        XCTAssertEqual(Set(labels).count, labels.count, "AppIconMode のラベルに重複がある")
    }

    /// プレビュー用アセット名は light だけライト、その他はダークを指す。
    func testAppIconModePreviewAsset() {
        XCTAssertEqual(AppIconMode.light.previewAsset, "AppIconLight")
        XCTAssertEqual(AppIconMode.dark.previewAsset, "AppIconDark")
        XCTAssertEqual(AppIconMode.auto.previewAsset, "AppIconDark")
    }

    // MARK: - AgentStatus.label

    /// AgentStatus は CaseIterable ではないため全ケースを明示列挙し、各 `.label` が
    /// 非空であること、かつラベルが一意（ピル表示の判別に使う）であることを確認する。
    func testAgentStatusLabelsAreNonEmptyAndDistinct() {
        let all: [AgentStatus] = [.none, .starting, .running, .waitingForInput, .idle, .ended]
        let labels = all.map(\.label)
        for (status, label) in zip(all, labels) {
            XCTAssertFalse(label.isEmpty, "AgentStatus.label が空: \(status)")
        }
        XCTAssertEqual(Set(labels).count, labels.count, "AgentStatus のラベルに重複がある")
    }

    // MARK: - RepoPalette

    /// パレットのエントリは複数あり、id/name は非空で id は一意（永続化キーに使う）。
    func testRepoPaletteEntriesAreWellFormed() {
        let entries = RepoPalette.entries
        XCTAssertFalse(entries.isEmpty, "RepoPalette.entries が空")
        for entry in entries {
            XCTAssertFalse(entry.id.isEmpty, "RepoPalette エントリの id が空")
            XCTAssertFalse(entry.name.isEmpty, "RepoPalette エントリの name が空")
        }
        let ids = entries.map(\.id)
        XCTAssertEqual(Set(ids).count, ids.count, "RepoPalette の id に重複がある")
    }

    /// 既知 id の参照は対応エントリの色を返し、同じ入力に対して安定（決定的）である。
    func testRepoPaletteColorLookupMatchesEntries() {
        for entry in RepoPalette.entries {
            XCTAssertEqual(
                RepoPalette.color(for: entry.id), entry.color,
                "color(for:) が id=\(entry.id) のエントリ色と一致しない"
            )
            // 同じ入力に対して同値を返す（呼び出しごとに揺れない）。
            XCTAssertEqual(RepoPalette.color(for: entry.id), RepoPalette.color(for: entry.id))
        }
    }

    /// nil や未知 id はフォールバック色（.secondary）を返し、両者は同一。
    func testRepoPaletteColorLookupFallback() {
        let fallbackForNil = RepoPalette.color(for: nil)
        let fallbackForUnknown = RepoPalette.color(for: "this-id-does-not-exist")
        XCTAssertEqual(fallbackForNil, Color.secondary)
        XCTAssertEqual(fallbackForUnknown, Color.secondary)
        XCTAssertEqual(fallbackForNil, fallbackForUnknown)
    }
}
