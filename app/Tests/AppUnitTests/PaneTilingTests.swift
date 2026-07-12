import XCTest
import AppKit
@testable import LaboLabo

/// `PaneTilingModel` は「1 セッション = 1 タイル木」を表す純粋なツリー/レイアウト
/// ロジック。SwiftUI/AppKit 描画（TilingCoordinator・端末サーフェス）に依存しない
/// 木の操作（split/add/close/swap・スナップショット復元）と `PaneKind` の既定タイトルを
/// 検証する。`@MainActor` 型なのでクラスごと main actor に隔離する。
@MainActor
final class PaneTilingTests: XCTestCase {

    // MARK: - fixtures

    /// 単一リーフ（root がそのままペイン）のモデルを実イニシャライザから組む。
    private func makeSinglePaneModel(kind: PaneKind = .terminal) -> PaneTilingModel {
        PaneTilingModel(root: TileNode(pane: PaneItem(kind: kind, title: kind.defaultTitle)))
    }

    // MARK: - PaneKind.defaultTitle

    /// 各 case が想定どおりのタイトルを返す。ローカライズ対象（terminal/files/commits）は
    /// 同一プロセス内で同じ `String(localized:)` 解決になるため、キーから算出した期待値と
    /// 一致するはず。diff だけはリテラル "Diff"。
    func testDefaultTitlePerKind() {
        XCTAssertEqual(PaneKind.terminal.defaultTitle, String(localized: "端末"))
        XCTAssertEqual(PaneKind.files.defaultTitle, String(localized: "変更ファイル"))
        XCTAssertEqual(PaneKind.commits.defaultTitle, String(localized: "履歴"))
        XCTAssertEqual(PaneKind.diff.defaultTitle, "Diff")
    }

    /// 4 種のタイトルは互いに異なり、いずれも空でない（復元時のフォールバックに使うため）。
    func testDefaultTitlesAreDistinctAndNonEmpty() {
        let titles = [
            PaneKind.terminal.defaultTitle,
            PaneKind.files.defaultTitle,
            PaneKind.diff.defaultTitle,
            PaneKind.commits.defaultTitle,
        ]
        XCTAssertEqual(Set(titles).count, 4, "既定タイトルは 4 種すべて異なるべき")
        XCTAssertTrue(titles.allSatisfy { !$0.isEmpty })
    }

    /// rawValue は Codable の保存キーになるので固定であることを確認。
    func testPaneKindRawValuesAreStable() {
        XCTAssertEqual(PaneKind.terminal.rawValue, "terminal")
        XCTAssertEqual(PaneKind.files.rawValue, "files")
        XCTAssertEqual(PaneKind.diff.rawValue, "diff")
        XCTAssertEqual(PaneKind.commits.rawValue, "commits")
        XCTAssertEqual(PaneKind(rawValue: "commits"), .commits)
        XCTAssertNil(PaneKind(rawValue: "unknown"))
    }

    // MARK: - defaultLayout

    /// 既定配置は terminal / commits / files / diff の 4 ペインを（木の DFS 順で）持つ。
    func testDefaultLayoutContainsExpectedPaneKinds() {
        let model = PaneTilingModel.defaultLayout()
        XCTAssertEqual(model.panes.count, 4)
        XCTAssertEqual(model.panes.map(\.kind), [.terminal, .commits, .files, .diff])
        for kind in [PaneKind.terminal, .commits, .files, .diff] {
            XCTAssertTrue(model.hasPane(kind: kind), "\(kind) が既定配置に含まれるべき")
        }
    }

    /// 既定配置のルートは上=端末 / 下=行 の縦 2 分割（ratio 0.55）。
    func testDefaultLayoutRootStructure() {
        let model = PaneTilingModel.defaultLayout()
        XCTAssertFalse(model.root.isLeaf)
        XCTAssertEqual(model.root.orientation, .vertical)
        XCTAssertEqual(model.root.ratio, 0.55, accuracy: 0.0001)
        XCTAssertEqual(model.root.children.count, 2)
        // ルート第 1 子は端末リーフ。
        XCTAssertTrue(model.root.children[0].isLeaf)
        XCTAssertEqual(model.root.children[0].selectedPane?.kind, .terminal)
    }

    // MARK: - split / add / close

    /// リーフを split するとペイン数が 1→2 に増え、そのノードが分割ノードになる。
    func testSplitIncreasesPaneCount() {
        let model = makeSinglePaneModel(kind: .terminal)
        XCTAssertEqual(model.panes.count, 1)
        let leafID = model.panes[0].id

        model.split(paneID: leafID, orientation: .horizontal, newPane: PaneItem(kind: .files, title: "f"))

        XCTAssertEqual(model.panes.count, 2)
        XCTAssertFalse(model.root.isLeaf)
        XCTAssertEqual(model.root.orientation, .horizontal)
        XCTAssertEqual(model.root.children.count, 2)
        XCTAssertTrue(model.hasPane(kind: .files))
        XCTAssertTrue(model.hasPane(kind: .terminal))
    }

    /// 存在しない paneID への split は何もしない（no-op）。
    func testSplitUnknownPaneIsNoOp() {
        let model = makeSinglePaneModel()
        model.split(paneID: UUID(), orientation: .vertical, newPane: PaneItem(kind: .files, title: "f"))
        XCTAssertEqual(model.panes.count, 1)
        XCTAssertTrue(model.root.isLeaf)
    }

    /// addPane はルートを分割ノードで包み、ペイン数を 1 増やす。
    func testAddPaneIncreasesPaneCount() {
        let model = makeSinglePaneModel(kind: .terminal)
        model.addPane(PaneItem(kind: .diff, title: "Diff"))
        XCTAssertEqual(model.panes.count, 2)
        XCTAssertTrue(model.hasPane(kind: .diff))
    }

    /// close は 2→1 に減らし、閉じた側の兄弟が残る。
    func testCloseDecreasesPaneCountAndKeepsSibling() {
        let model = makeSinglePaneModel(kind: .terminal)
        model.addPane(PaneItem(kind: .files, title: "f"))
        XCTAssertEqual(model.panes.count, 2)
        let terminalID = model.panes[0].id
        XCTAssertEqual(model.panes[0].kind, .terminal)

        model.close(paneID: terminalID)

        XCTAssertEqual(model.panes.count, 1)
        XCTAssertEqual(model.panes[0].kind, .files, "閉じたペインの兄弟（files）が残るべき")
    }

    /// 唯一のルートペインは閉じられない（最低 1 ペインを維持）。
    func testCloseRootOnlyPaneIsNoOp() {
        let model = makeSinglePaneModel(kind: .terminal)
        let onlyID = model.panes[0].id
        model.close(paneID: onlyID)
        XCTAssertEqual(model.panes.count, 1, "ルート単独ペインは残す")
        XCTAssertEqual(model.panes[0].kind, .terminal)
    }

    // MARK: - addPaneIfAbsent

    /// addPaneIfAbsent は同種ペインが既にあれば追加しない（重複させない）。
    func testAddPaneIfAbsentDoesNotDuplicate() {
        let model = makeSinglePaneModel(kind: .terminal)
        // 端末は既にあるので増えない。
        model.addPaneIfAbsent(kind: .terminal, title: "dup")
        XCTAssertEqual(model.panes.count, 1)

        // files は未存在なので 1 増える。
        model.addPaneIfAbsent(kind: .files, title: "f")
        XCTAssertEqual(model.panes.count, 2)
        XCTAssertEqual(model.panes.filter { $0.kind == .files }.count, 1)

        // 2 度目の files は重複しない。
        model.addPaneIfAbsent(kind: .files, title: "f2")
        XCTAssertEqual(model.panes.count, 2)
        XCTAssertEqual(model.panes.filter { $0.kind == .files }.count, 1)
    }

    // MARK: - snapshot / apply (シリアライズ往復)

    /// snapshot → model(from:) の往復でペイン種別とルート構造が保たれる。
    func testSnapshotRoundTripPreservesLayout() {
        let original = PaneTilingModel.defaultLayout()
        let layout = original.snapshot()

        let restored = PaneTilingModel.model(from: layout)
        XCTAssertNotNil(restored)
        XCTAssertEqual(restored?.panes.map(\.kind), original.panes.map(\.kind))
        XCTAssertEqual(restored?.root.orientation, .vertical)
        XCTAssertEqual(restored?.root.ratio ?? -1, 0.55, accuracy: 0.0001)
    }

    /// apply は現在のツリーを保存表現で丸ごと差し替える。
    func testApplyReplacesTree() {
        let model = makeSinglePaneModel(kind: .terminal)
        let target = PaneTilingModel.defaultLayout().snapshot()
        model.apply(target)
        XCTAssertEqual(model.panes.map(\.kind), [.terminal, .commits, .files, .diff])
    }

    /// 不正なレイアウト（子が 2 未満の分割）は復元できず nil。
    func testModelFromInvalidLayoutReturnsNil() {
        let broken = TileLayout(orientation: "horizontal", ratio: 0.5, children: [])
        XCTAssertNil(PaneTilingModel.model(from: broken))
    }

    /// resetToDefault で既定 4 ペイン配置に戻る。
    func testResetToDefaultRestoresFourPanes() {
        let model = makeSinglePaneModel(kind: .terminal)
        model.addPane(PaneItem(kind: .files, title: "f"))
        model.resetToDefault()
        XCTAssertEqual(model.panes.count, 4)
        XCTAssertEqual(model.panes.map(\.kind), [.terminal, .commits, .files, .diff])
    }

    // MARK: - タブ（中央ドロップ合流 / 端ドロップで分割独立 / タブ単位クローズ）

    /// 中央ドロップで別ペインのタブグループへ合流し、合流したタブが選択される。
    func testMoveCenterMergesIntoTabGroup() {
        let model = makeSinglePaneModel(kind: .terminal)
        model.addPane(PaneItem(kind: .files, title: "f"))
        let terminalID = model.panes[0].id
        let filesID = model.panes[1].id

        model.move(terminalID, toEdgeOf: filesID, edge: .center)

        XCTAssertEqual(model.panes.count, 2, "タブ合流でペイン総数は変わらない")
        XCTAssertTrue(model.root.isLeaf, "リーフ 2 枚が 1 つのタブグループに畳まれる")
        XCTAssertEqual(model.root.panes.map(\.kind), [.files, .terminal])
        XCTAssertEqual(model.root.selectedPane?.id, terminalID, "合流したタブが選択される")
    }

    /// グループ内のタブを端へドロップすると分割で独立する。
    func testMoveEdgeSplitsTabOutOfGroup() {
        let model = makeSinglePaneModel(kind: .terminal)
        model.addPane(PaneItem(kind: .files, title: "f"))
        let terminalID = model.panes[0].id
        let filesID = model.panes[1].id
        model.move(terminalID, toEdgeOf: filesID, edge: .center) // まず合流

        model.move(terminalID, toEdgeOf: filesID, edge: .right) // 右へ分割独立

        XCTAssertFalse(model.root.isLeaf)
        XCTAssertEqual(model.panes.count, 2)
        XCTAssertEqual(model.root.children[0].selectedPane?.kind, .files)
        XCTAssertEqual(model.root.children[1].selectedPane?.kind, .terminal)
    }

    /// 複数タブのグループで close はそのタブだけ閉じ、前面になるタブの id を返す。
    func testCloseTabInGroupKeepsSiblingTabs() {
        let model = makeSinglePaneModel(kind: .terminal)
        model.addPane(PaneItem(kind: .files, title: "f"))
        let terminalID = model.panes[0].id
        let filesID = model.panes[1].id
        model.move(terminalID, toEdgeOf: filesID, edge: .center)

        let revealed = model.close(paneID: terminalID)

        XCTAssertEqual(model.panes.count, 1)
        XCTAssertEqual(model.panes[0].kind, .files)
        XCTAssertEqual(revealed, filesID, "閉じた結果前面になるタブが返る")
    }

    // MARK: - タブグループの保存 / 復元

    /// タブグループ（2 枚以上）は panes/selectedIndex で往復し、
    /// タブ別のセッション ID / transcript パスも保たれる。
    func testTabGroupSnapshotRoundTrip() {
        let model = makeSinglePaneModel(kind: .terminal)
        model.addPane(PaneItem(kind: .terminal, title: "t2"))
        let a = model.panes[0].id
        let b = model.panes[1].id
        model.move(a, toEdgeOf: b, edge: .center)
        model.panes[0].agentSessionID = "sid-1"
        model.panes[0].agentTranscriptPath = "/tmp/t1.jsonl"

        let restored = PaneTilingModel.model(from: model.snapshot())

        XCTAssertNotNil(restored)
        XCTAssertTrue(restored?.root.isLeaf ?? false)
        XCTAssertEqual(restored?.root.panes.count, 2)
        XCTAssertEqual(restored?.root.panes[0].agentSessionID, "sid-1")
        XCTAssertEqual(restored?.root.panes[0].agentTranscriptPath, "/tmp/t1.jsonl")
        XCTAssertEqual(restored?.root.selectedIndex, model.root.selectedIndex)
    }

    /// 単一タブのリーフは旧形式（paneKind）で書き出し、旧データも読める（後方互換）。
    func testSingleTabEncodesLegacyFormatAndDecodesIt() {
        let model = makeSinglePaneModel(kind: .terminal)
        model.panes[0].agentSessionID = "sid-9"
        let layout = model.snapshot()
        XCTAssertEqual(layout.paneKind, "terminal", "単一タブは旧形式キーで書く")
        XCTAssertNil(layout.panes)
        XCTAssertEqual(layout.paneAgentSessionId, "sid-9")

        // 旧形式（タブ導入前の JSON 相当）も復元できる。
        let legacy = TileLayout(paneKind: "files", paneTitle: "変更")
        let restored = PaneTilingModel.model(from: legacy)
        XCTAssertEqual(restored?.root.selectedPane?.kind, .files)
        XCTAssertEqual(restored?.root.selectedPane?.title, "変更")
    }

    /// strippingAgentSessions はタブ別のセッション情報だけ除き、配置の形は保つ。
    func testStrippingAgentSessionsRemovesIDs() {
        let model = makeSinglePaneModel(kind: .terminal)
        model.addPane(PaneItem(kind: .terminal, title: "t2", agentSessionID: "sid-2", agentTranscriptPath: "/tmp/x"))
        model.panes[0].agentSessionID = "sid-1"

        let restored = PaneTilingModel.model(from: model.snapshot().strippingAgentSessions())

        XCTAssertEqual(restored?.panes.count, 2)
        XCTAssertTrue(restored?.panes.allSatisfy { $0.agentSessionID == nil } ?? false)
        XCTAssertTrue(restored?.panes.allSatisfy { $0.agentTranscriptPath == nil } ?? false)
    }

    // MARK: - revision

    /// 構造変更ごとに revision がインクリメントされ、onLayoutChanged が呼ばれる。
    func testMutationBumpsRevisionAndFiresLayoutCallback() {
        let model = makeSinglePaneModel(kind: .terminal)
        XCTAssertEqual(model.revision, 0)

        var callCount = 0
        model.onLayoutChanged = { callCount += 1 }

        model.addPane(PaneItem(kind: .files, title: "f"))
        XCTAssertEqual(model.revision, 1)
        XCTAssertEqual(callCount, 1)

        let leafID = model.panes[0].id
        model.split(paneID: leafID, orientation: .vertical, newPane: PaneItem(kind: .diff, title: "Diff"))
        XCTAssertEqual(model.revision, 2)
        XCTAssertEqual(callCount, 2)
    }
}
