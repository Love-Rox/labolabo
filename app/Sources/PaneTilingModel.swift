import Foundation
import Observation

// MARK: - モデル層（UI 非依存 / 将来 Rust 移植の一次仕様）
//
// 1 セッションの作業領域は「二分タイル木」1 本で表す。葉（リーフ）は *タブグループ* で、
// 1 枚以上のペイン（端末サーフェス・変更ファイル・Diff・履歴グラフ）をタブとして束ね、
// 1 枚を選択している。分割ノードは 2 つの子を向き（orientation）と比率（ratio）で並べる。
//
// このファイルは AppKit / SwiftUI に一切依存しない（import は Foundation と Observation
// のみ）。端末 pty / スクロールバックの温存は AppKit 側（TilingCoordinator が葉 NSView を
// *reparent* するだけ）で実現しており、本モデルは純粋な木の操作と保存フォーマットだけを持つ。
// UI との結合点は 2 つだけ:
//   - 向き: モデルは AppKit 非依存の `TileOrientation` を使い、AppKit の
//     `NSUserInterfaceLayoutOrientation` との相互変換は UI 層（PaneTiling.swift）の境界に置く。
//   - 操作面: 端末へのテキスト送信とフォーカス移動だけを `PaneTilingActions` protocol へ
//     切り出し、AppKit 側の `TilingCoordinator` がこれに準拠する。
//
// 保存フォーマット（TileLayout の Codable 表現）と、pty 温存タブ・selectedIndex クランプ・
// 単一タブの旧形式シリアライズといった hard-won な挙動は、将来ほぼ逐語で Rust へ移植する
// 一次仕様。挙動・キー名・省略規則は変えないこと。

/// 分割ノードの子を並べる向き。AppKit の `NSUserInterfaceLayoutOrientation` に相当するが、
/// モデル層を UI 非依存に保つための自前表現。AppKit との相互変換は UI 層の境界で行う。
enum TileOrientation: Sendable {
    case horizontal, vertical
}

enum PaneKind: String {
    case terminal
    case files
    case diff
    case commits

    /// 復元時にタイトルが欠けていた場合の既定タイトル。
    var defaultTitle: String {
        switch self {
        case .terminal: return String(localized: "端末")
        case .files: return String(localized: "変更ファイル")
        case .diff: return "Diff"
        case .commits: return String(localized: "履歴")
        }
    }

    /// ヘッダー / タブチップ共用の SF Symbol 名。
    var iconName: String {
        switch self {
        case .terminal: return "terminal"
        case .files: return "list.bullet.rectangle"
        case .diff: return "doc.text"
        case .commits: return "point.3.connected.trianglepath.dotted"
        }
    }
}

/// タブ 1 枚分の保存表現。
struct PanePayload: Codable, Equatable {
    var kind: String
    var title: String?
    /// この端末タブで最後に観測した Claude セッション ID（タブ別 --resume 用）。
    var agentSessionId: String?
    /// 対応する transcript(JSONL) のパス。resume 前の実在チェックに使う（保存前に
    /// 終了した空セッションの ID へ無駄打ちしないため）。
    var agentTranscriptPath: String?
}

/// タイル配置の保存表現（Codable）。leaf は単一タブなら旧形式（`paneKind`/`paneTitle`）、
/// 2 枚以上のタブグループなら `panes`/`selectedIndex`。split は `orientation`/`ratio`/`children`。
/// セッション別配置と名前付きプリセットに使う。
struct TileLayout: Codable, Equatable {
    var paneKind: String?
    var paneTitle: String?
    /// 旧形式（単一タブ）リーフ用の Claude セッション ID。旧リーダーには未知キーとして無害。
    var paneAgentSessionId: String?
    /// 旧形式（単一タブ）リーフ用の transcript パス。
    var paneAgentTranscriptPath: String?
    /// タブグループ（タブが 2 枚以上のときだけ書き出す。後方互換のため単一タブは旧形式）。
    var panes: [PanePayload]?
    /// 選択中タブの index（`panes` を書いたときだけ意味を持つ）。
    var selectedIndex: Int?
    var orientation: String?
    var ratio: CGFloat?
    var children: [TileLayout]?

    init(
        paneKind: String? = nil, paneTitle: String? = nil, paneAgentSessionId: String? = nil,
        paneAgentTranscriptPath: String? = nil,
        panes: [PanePayload]? = nil, selectedIndex: Int? = nil,
        orientation: String? = nil, ratio: CGFloat? = nil, children: [TileLayout]? = nil
    ) {
        self.paneKind = paneKind
        self.paneTitle = paneTitle
        self.paneAgentSessionId = paneAgentSessionId
        self.paneAgentTranscriptPath = paneAgentTranscriptPath
        self.panes = panes
        self.selectedIndex = selectedIndex
        self.orientation = orientation
        self.ratio = ratio
        self.children = children
    }

    /// タブ別の Claude セッション情報（ID / transcript パス）を除いたコピー。プリセットは
    /// 全セッション共通の「配置の形」なので、特定セッションの再開情報を持ち込まないために使う。
    func strippingAgentSessions() -> TileLayout {
        var copy = self
        copy.paneAgentSessionId = nil
        copy.paneAgentTranscriptPath = nil
        copy.panes = copy.panes?.map { payload in
            var stripped = payload
            stripped.agentSessionId = nil
            stripped.agentTranscriptPath = nil
            return stripped
        }
        copy.children = copy.children?.map { $0.strippingAgentSessions() }
        return copy
    }
}

/// 名前付きレイアウトプリセット（全セッション共通）。
struct LayoutPreset: Codable, Equatable, Identifiable {
    var name: String
    var layout: TileLayout
    var id: String { name }
}

@MainActor
@Observable
final class PaneItem: Identifiable {
    let id = UUID()
    let kind: PaneKind
    var title: String
    /// この端末タブで最後に観測した Claude セッション ID（hooks の labolabo_pane_id 対応付け）。
    /// レイアウトと一緒に保存し、次回起動のタブ別 --resume に使う。端末以外は常に nil。
    var agentSessionID: String?
    /// 対応する transcript(JSONL) のパス（hooks 由来）。resume 前の実在チェック用。
    var agentTranscriptPath: String?

    init(kind: PaneKind, title: String, agentSessionID: String? = nil, agentTranscriptPath: String? = nil) {
        self.kind = kind
        self.title = title
        self.agentSessionID = agentSessionID
        self.agentTranscriptPath = agentTranscriptPath
    }
}

/// A node in the binary tile tree: a leaf (tab group, `panes` non-empty) or a
/// 2-way split (`panes` empty, two `children`).
@MainActor
@Observable
final class TileNode: Identifiable {
    let id = UUID()
    /// リーフのタブ群（空 = split ノード）。並び順 = タブバーの表示順。
    var panes: [PaneItem]
    /// 選択中タブの index。panes を変えたら必ず有効範囲へクランプする（範囲外だと
    /// selectedPane が first に退避してしまい、表示と保存がずれる）。
    var selectedIndex: Int
    var orientation: TileOrientation
    var children: [TileNode]
    /// First child's fraction of the split (0…1). Persisted across rebuilds.
    var ratio: CGFloat

    init(pane: PaneItem) {
        panes = [pane]
        selectedIndex = 0
        orientation = .horizontal
        children = []
        ratio = 0.5
    }

    /// グループごと（タブ構成 + 選択）を新しいノードへ移すとき用。
    init(panes: [PaneItem], selectedIndex: Int) {
        self.panes = panes
        self.selectedIndex = selectedIndex
        orientation = .horizontal
        children = []
        ratio = 0.5
    }

    init(orientation: TileOrientation, ratio: CGFloat, children: [TileNode]) {
        panes = []
        selectedIndex = 0
        self.orientation = orientation
        self.ratio = ratio
        self.children = children
    }

    var isLeaf: Bool { !panes.isEmpty }

    /// 表示中タブ。selectedIndex が壊れていても first に退避して 1 枚は必ず返す。
    var selectedPane: PaneItem? {
        panes.indices.contains(selectedIndex) ? panes[selectedIndex] : panes.first
    }
}

/// Where a dragged pane is dropped relative to the target pane.
enum DropEdge {
    case left, right, top, bottom, center
}

/// モデルが UI（AppKit コーディネータ）へ依頼する操作面。モデル層を AppKit 非依存に保つため、
/// 端末へのテキスト送信と「次の再構築後にフォーカスを渡すペイン」の指定だけを切り出す。
/// AppKit 側の `TilingCoordinator` がこれに準拠する。現状使われている操作のみを持つ。
@MainActor
protocol PaneTilingActions: AnyObject {
    /// 次の reconcile 後にキーフォーカスを渡す端末ペイン。
    var pendingFocusPaneID: UUID? { get set }
    /// 指定ペインの端末へテキスト（コマンド）を送る。
    func scheduleSend(paneID: UUID, text: String)
}

@MainActor
@Observable
final class PaneTilingModel {
    var root: TileNode
    /// Bumped on every structural mutation so SwiftUI re-invokes updateNSView.
    private(set) var revision: Int = 0
    /// UI（AppKit コーディネータ）への操作面。端末への text 送信・フォーカス移動に使う。
    weak var coordinator: PaneTilingActions?
    /// 構造変更（追加/削除/分割/移動/合流）とタブ選択のたびに呼ばれる。配置の永続化に使う。
    /// ratio（ドラッグ）は bump しないので、離脱時に別途 snapshot して保存する。
    @ObservationIgnored var onLayoutChanged: (() -> Void)?

    init(root: TileNode) { self.root = root }

    /// 新しい端末ペインを作り、シェル起動後にコマンドを送る（Claude 起動などに使用）。
    func launchInNewTerminal(title: String, command: String) {
        let pane = PaneItem(kind: .terminal, title: title)
        addPane(pane)
        // 開いた端末でそのまま操作できるよう、再構築後にフォーカスを渡す。
        coordinator?.pendingFocusPaneID = pane.id
        coordinator?.scheduleSend(paneID: pane.id, text: command)
    }

    /// 既存の端末ペイン（最初に見つかったもの）へコマンドを送る。無ければ false。
    /// 自動 resume 用: 新規ペインを増やさず、復元直後のシェルに resume を打ち込む。
    func sendToExistingTerminal(command: String) -> Bool {
        guard let pane = panes.first(where: { $0.kind == .terminal }) else { return false }
        coordinator?.scheduleSend(paneID: pane.id, text: command)
        return true
    }

    /// 端末タブ（全リーフ横断・木の順）。タブ別 resume の走査に使う。
    var terminalPanes: [PaneItem] { panes.filter { $0.kind == .terminal } }

    /// 指定ペインの端末へコマンドを送る（タブ別 resume 用）。
    func sendToTerminal(paneID: UUID, command: String) {
        coordinator?.scheduleSend(paneID: paneID, text: command)
    }

    /// hooks 由来の（ペイン, Claude セッション ID, transcript パス）対応を記録し、
    /// レイアウトと一緒に永続化する。構造は変わらないので bump しない（reconcile 不要）。
    func recordAgentSession(id: String, paneUUIDString: String, transcriptPath: String?) {
        guard let uuid = UUID(uuidString: paneUUIDString),
              let pane = panes.first(where: { $0.id == uuid }),
              pane.kind == .terminal else { return }
        let newTranscript = transcriptPath ?? pane.agentTranscriptPath
        guard pane.agentSessionID != id || pane.agentTranscriptPath != newTranscript else { return }
        pane.agentSessionID = id
        pane.agentTranscriptPath = newTranscript
        onLayoutChanged?()
    }

    static func defaultLayout() -> PaneTilingModel {
        // Terminal on top; bottom row = commit graph | changed-files | diff (1:1:2).
        let terminal = TileNode(pane: PaneItem(kind: .terminal, title: String(localized: "端末")))
        let commits = TileNode(pane: PaneItem(kind: .commits, title: String(localized: "履歴")))
        let files = TileNode(pane: PaneItem(kind: .files, title: String(localized: "変更ファイル")))
        let diff = TileNode(pane: PaneItem(kind: .diff, title: "Diff"))
        // files : diff = 1 : 2 → files takes 1/3 of (files+diff)
        let filesAndDiff = TileNode(orientation: .horizontal, ratio: 1.0 / 3.0, children: [files, diff])
        // commits : (files+diff) = 1 : 3 → commits takes 1/4 of the bottom row
        let bottom = TileNode(orientation: .horizontal, ratio: 0.25, children: [commits, filesAndDiff])
        let root = TileNode(orientation: .vertical, ratio: 0.55, children: [terminal, bottom])
        return PaneTilingModel(root: root)
    }

    private func bump() {
        revision &+= 1
        onLayoutChanged?()
    }

    /// 全リーフの全タブ。非表示タブも含めることで reconcile の liveIDs（contentCache の
    /// purge 判定）に残り、隠れているタブの pty が殺されない。
    var panes: [PaneItem] { Self.leaves(root).flatMap(\.panes) }
    func hasPane(kind: PaneKind) -> Bool { panes.contains { $0.kind == kind } }

    // MARK: - 配置のシリアライズ / 復元 / プリセット適用

    /// 保存済みの配置からモデルを組み立てる（不正なら nil）。
    static func model(from layout: TileLayout) -> PaneTilingModel? {
        guard let node = decode(layout) else { return nil }
        return PaneTilingModel(root: node)
    }

    /// 現在のタイル木を保存用スナップショットへ。
    func snapshot() -> TileLayout { Self.encode(root) }

    /// 保存済みの配置（プリセット/セッション別）を適用してツリーを差し替える。
    func apply(_ layout: TileLayout) {
        guard let node = Self.decode(layout) else { return }
        root = node
        bump()
    }

    /// 既定配置へ戻す。
    func resetToDefault() {
        root = Self.defaultLayout().root
        bump()
    }

    private static func encode(_ node: TileNode) -> TileLayout {
        if node.isLeaf {
            // 後方互換: 単一タブは旧形式（paneKind/paneTitle）で書く。タブ導入前のリーダーでも
            // 読めるよう、2 枚以上のときだけ新しい panes/selectedIndex を使う。
            if node.panes.count == 1, let pane = node.panes.first {
                return TileLayout(
                    paneKind: pane.kind.rawValue,
                    paneTitle: pane.title,
                    paneAgentSessionId: pane.agentSessionID,
                    paneAgentTranscriptPath: pane.agentTranscriptPath
                )
            }
            return TileLayout(
                panes: node.panes.map {
                    PanePayload(
                        kind: $0.kind.rawValue, title: $0.title,
                        agentSessionId: $0.agentSessionID,
                        agentTranscriptPath: $0.agentTranscriptPath
                    )
                },
                selectedIndex: node.selectedIndex
            )
        }
        return TileLayout(
            orientation: node.orientation == .vertical ? "vertical" : "horizontal",
            ratio: node.ratio,
            children: node.children.map(encode)
        )
    }

    private static func decode(_ layout: TileLayout) -> TileNode? {
        // 新形式のタブグループを優先。不正 kind の要素は捨て、全滅ならリーフ不成立で nil。
        if let payloads = layout.panes {
            let items = payloads.compactMap { payload -> PaneItem? in
                guard let kind = PaneKind(rawValue: payload.kind) else { return nil }
                return PaneItem(
                    kind: kind,
                    title: payload.title ?? kind.defaultTitle,
                    agentSessionID: payload.agentSessionId,
                    agentTranscriptPath: payload.agentTranscriptPath
                )
            }
            guard !items.isEmpty else { return nil }
            let idx = min(max(0, layout.selectedIndex ?? 0), items.count - 1)
            return TileNode(panes: items, selectedIndex: idx)
        }
        // 旧形式: 単一リーフ。
        if let kindRaw = layout.paneKind, let kind = PaneKind(rawValue: kindRaw) {
            return TileNode(pane: PaneItem(
                kind: kind,
                title: layout.paneTitle ?? kind.defaultTitle,
                agentSessionID: layout.paneAgentSessionId,
                agentTranscriptPath: layout.paneAgentTranscriptPath
            ))
        }
        guard let children = layout.children, children.count >= 2 else { return nil }
        let nodes = children.compactMap(decode)
        guard nodes.count == children.count else { return nil }
        let orientation: TileOrientation =
            layout.orientation == "vertical" ? .vertical : .horizontal
        let ratio = min(0.95, max(0.05, layout.ratio ?? 0.5))
        return TileNode(orientation: orientation, ratio: ratio, children: nodes)
    }

    // MARK: mutations

    func split(paneID: UUID, orientation: TileOrientation, newPane: PaneItem) {
        guard let node = Self.findLeaf(containing: paneID, in: root) else { return }
        // keep 側はグループ全体（タブ構成/選択を引き継ぐ）、added 側は新ペイン単独。
        splitLeafOut(node, movedPane: newPane, edge: orientation == .horizontal ? .right : .bottom)
        bump()
    }

    func addPane(_ pane: PaneItem) {
        let added = TileNode(pane: pane)
        let newRoot = TileNode(orientation: .horizontal, ratio: 0.7, children: [root, added])
        root = newRoot
        bump()
    }

    func addPaneIfAbsent(kind: PaneKind, title: String) {
        guard !hasPane(kind: kind) else { return }
        addPane(PaneItem(kind: kind, title: title))
    }

    /// タブ/ペインを閉じる。戻り値は「閉じた結果、新しく前面になったタブ」の id
    ///（フォーカス移動に使う。閉じられなかった/前面が定まらないときは nil）。
    @discardableResult
    func close(paneID: UUID) -> UUID? {
        guard let node = Self.findLeaf(containing: paneID, in: root),
              let index = node.panes.firstIndex(where: { $0.id == paneID }) else { return nil }
        if node.panes.count > 1 {
            // グループに複数タブ → その 1 枚だけ閉じる（残タブの pty は温存される）。
            removePane(at: index, from: node)
            bump()
            return node.selectedPane?.id
        }
        // 最後の 1 枚: 従来どおり親を兄弟へ collapse。root 単独リーフなら最低 1 ペインを維持して no-op。
        guard let (parent, pIndex) = Self.findParent(root, childID: node.id) else { return nil }
        collapse(parent, into: parent.children[pIndex == 0 ? 1 : 0])
        bump()
        // collapse 後の parent がリーフ（タブグループ）ならその選択タブが前面になる。
        return parent.selectedPane?.id
    }

    /// タブ/ペインの移動。実際に木を変更したら true（フォーカス移動の判定に使う）。
    @discardableResult
    func move(_ sourceID: UUID, toEdgeOf targetID: UUID, edge: DropEdge) -> Bool {
        guard let sourceLeaf = Self.findLeaf(containing: sourceID, in: root),
              let sourceIndex = sourceLeaf.panes.firstIndex(where: { $0.id == sourceID }),
              let targetLeaf = Self.findLeaf(containing: targetID, in: root) else { return false }
        let source = sourceLeaf.panes[sourceIndex]

        // 同一グループへのドロップ。
        if sourceLeaf === targetLeaf {
            if edge == .center { return false }                     // 合流済み（同じグループの中央）= no-op
            guard sourceLeaf.panes.count > 1 else { return false }  // 単独タブを自分の端に落とすのは無意味
            removePane(at: sourceIndex, from: sourceLeaf)
            // 残ったグループを edge 方向へ 2 分割し、source を新リーフとして edge 側へ独立させる。
            splitLeafOut(sourceLeaf, movedPane: source, edge: edge)
            bump()
            return true
        }

        // 別グループへ: source を元グループから取り除く（空になれば親を collapse で畳む）。
        removePane(at: sourceIndex, from: sourceLeaf)
        if sourceLeaf.panes.isEmpty { detach(sourceLeaf) }

        // 木が変わったので target を取り直す（消えていれば現行 move と同じく bump して return）。
        guard let target = Self.findLeaf(containing: targetID, in: root) else { bump(); return true }
        if edge == .center {
            // タブ合流: 末尾に足して選択。
            target.panes.append(source)
            target.selectedIndex = target.panes.count - 1
        } else {
            splitLeafOut(target, movedPane: source, edge: edge)
        }
        bump()
        return true
    }

    /// タブ選択の変更。構造は変わらない（reconcile 不要）ので bump しないが、選択状態の永続化の
    /// ため onLayoutChanged だけ呼ぶ。表示（isHidden）の張り替えは AppKit 側が行う。
    func selectTab(paneID: UUID) {
        guard let node = Self.findLeaf(containing: paneID, in: root),
              let i = node.panes.firstIndex(where: { $0.id == paneID }),
              node.selectedIndex != i else { return }
        node.selectedIndex = i
        onLayoutChanged?()
    }

    /// リーフ `node`（グループ全体を保持）を edge 方向に 2 分割し、`movedPane` を edge 側の
    /// 新リーフとして置く。keep 側は既存のタブ構成/選択を丸ごと引き継ぐ。
    private func splitLeafOut(_ node: TileNode, movedPane: PaneItem, edge: DropEdge) {
        let orientation: TileOrientation =
            (edge == .left || edge == .right) ? .horizontal : .vertical
        let movedOnSecond = (edge == .right || edge == .bottom)
        let keep = TileNode(panes: node.panes, selectedIndex: node.selectedIndex)
        let moved = TileNode(pane: movedPane)
        node.panes = []
        node.selectedIndex = 0
        node.orientation = orientation
        node.children = movedOnSecond ? [keep, moved] : [moved, keep]
        node.ratio = 0.5
    }

    /// リーフからタブ 1 枚を取り除き、selectedIndex を「見ていたタブが動かない」ように寄せる。
    /// 除去位置より前を選択中だったら index を 1 つ手前へずらす（後続タブが繰り上がるため）。
    private func removePane(at index: Int, from node: TileNode) {
        node.panes.remove(at: index)
        if index < node.selectedIndex { node.selectedIndex -= 1 }
        node.selectedIndex = node.panes.isEmpty ? 0 : min(node.selectedIndex, node.panes.count - 1)
    }

    /// Remove a leaf by collapsing its parent into the sibling (no bump).
    private func detach(_ node: TileNode) {
        guard let (parent, index) = Self.findParent(root, childID: node.id) else { return }
        collapse(parent, into: parent.children[index == 0 ? 1 : 0])
    }

    private func collapse(_ parent: TileNode, into sibling: TileNode) {
        parent.panes = sibling.panes
        parent.selectedIndex = sibling.selectedIndex
        parent.orientation = sibling.orientation
        parent.children = sibling.children
        parent.ratio = sibling.ratio
    }

    // MARK: tree helpers

    static func leaves(_ node: TileNode) -> [TileNode] {
        node.isLeaf ? [node] : node.children.flatMap(leaves)
    }

    /// paneID を含むリーフノードを返す。
    static func findLeaf(containing paneID: UUID, in node: TileNode) -> TileNode? {
        if node.isLeaf {
            return node.panes.contains(where: { $0.id == paneID }) ? node : nil
        }
        for child in node.children {
            if let found = findLeaf(containing: paneID, in: child) { return found }
        }
        return nil
    }

    static func findParent(_ node: TileNode, childID: UUID) -> (TileNode, Int)? {
        for (index, child) in node.children.enumerated() {
            if child.id == childID { return (node, index) }
            if let found = findParent(child, childID: childID) { return found }
        }
        return nil
    }
}
