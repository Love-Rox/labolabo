import SwiftUI
import AppKit
import Observation
import GhosttyTerminal

// MARK: - Model
//
// One session's work area is a single binary tile tree whose leaves are *tab
// groups*: each leaf holds one or more panes (a terminal surface, the
// changed-files pane, the diff pane, or the commit graph) stacked as tabs, with
// one selected. The AppKit layer (TilingCoordinator) owns the leaf NSViews and
// only *reparents* them between NSSplitViews as the tree changes, so a
// terminal's pty/scrollback survives split / move / tab-merge (AppTerminalView
// keeps its surface across reparenting). Dropping a pane on another leaf's
// center merges it in as a tab; dropping on an edge still splits.

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
    var orientation: NSUserInterfaceLayoutOrientation
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

    init(orientation: NSUserInterfaceLayoutOrientation, ratio: CGFloat, children: [TileNode]) {
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

/// Inputs the tiling needs to build leaf content.
struct PaneContext {
    let workingDirectory: String
    let work: WorkPaneModel
    let configSource: TerminalController.ConfigSource
}

@MainActor
@Observable
final class PaneTilingModel {
    var root: TileNode
    /// Bumped on every structural mutation so SwiftUI re-invokes updateNSView.
    private(set) var revision: Int = 0
    /// AppKit 側コーディネータ（端末への text 送信に使う）。
    weak var coordinator: TilingCoordinator?
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
        let orientation: NSUserInterfaceLayoutOrientation =
            layout.orientation == "vertical" ? .vertical : .horizontal
        let ratio = min(0.95, max(0.05, layout.ratio ?? 0.5))
        return TileNode(orientation: orientation, ratio: ratio, children: nodes)
    }

    // MARK: mutations

    func split(paneID: UUID, orientation: NSUserInterfaceLayoutOrientation, newPane: PaneItem) {
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
        let orientation: NSUserInterfaceLayoutOrientation =
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

// MARK: - SwiftUI bridge

struct PaneTilingView: NSViewRepresentable {
    let model: PaneTilingModel
    let context: PaneContext
    /// Pass `model.revision` so SwiftUI re-runs updateNSView on structural change.
    let revision: Int

    func makeCoordinator() -> TilingCoordinator {
        let coordinator = TilingCoordinator(model: model, context: context)
        model.coordinator = coordinator
        return coordinator
    }

    func makeNSView(context nsContext: Context) -> NSView {
        let container = NSView()
        nsContext.coordinator.container = container
        nsContext.coordinator.reconcile()
        return container
    }

    func updateNSView(_ nsView: NSView, context nsContext: Context) {
        nsContext.coordinator.reconcile()
    }
}

// MARK: - AppKit coordinator (owns leaf NSViews)

@MainActor
final class TilingCoordinator: NSObject {
    let model: PaneTilingModel
    let context: PaneContext
    weak var container: NSView?

    private var contentCache: [UUID: NSView] = [:]
    private var terminalDelegates: [UUID: TerminalLeafDelegate] = [:]
    /// 次の reconcile 後にキーフォーカスを渡す端末ペイン。構造変更（タブ移動・分割・
    /// 閉じる・新規端末）で「新しく前面になった端末」へ、木の再構築を待ってから
    /// フォーカスを移すために使う（再構築前に makeFirstResponder しても view が
    /// 入れ替わって無効になるため）。
    var pendingFocusPaneID: UUID?
    /// 最後に再構築した revision。構造が変わっていないのに毎回ツリーを作り直すと
    /// 端末がチラつくため、revision が変わったときだけ reconcile する。
    private var lastRevision: Int = -1

    init(model: PaneTilingModel, context: PaneContext) {
        self.model = model
        self.context = context
    }

    func reconcile() {
        guard let container else { return }
        // 構造未変更（revision 据え置き）なら再構築しない。SwiftUI の再評価（状態ドットの
        // パルス・git 更新・タブ選択の onLayoutChanged など）で updateNSView が走っても
        // ツリーを壊さず、チラつきを防ぐ。
        guard model.revision != lastRevision else { return }
        lastRevision = model.revision
        let liveIDs = Set(model.panes.map(\.id))

        let tree = buildNode(model.root)
        container.subviews.forEach { $0.removeFromSuperview() }
        tree.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(tree)
        NSLayoutConstraint.activate([
            tree.topAnchor.constraint(equalTo: container.topAnchor),
            tree.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            tree.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            tree.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])

        // Release content (and surfaces) for panes that no longer exist. `liveIDs` は
        // 非表示タブも含む全タブなので、隠れているだけのタブは purge されない。
        for id in contentCache.keys where !liveIDs.contains(id) {
            contentCache[id] = nil
            terminalDelegates[id] = nil
        }

        // 構造変更で新しく前面になった端末へフォーカスを渡す（AppKit の親付け完了後）。
        if let focusID = pendingFocusPaneID {
            pendingFocusPaneID = nil
            if let terminal = contentCache[focusID] as? AppTerminalView {
                DispatchQueue.main.async { [weak terminal] in
                    guard let terminal, let window = terminal.window else { return }
                    window.makeFirstResponder(terminal)
                }
            }
        }
    }

    private func buildNode(_ node: TileNode) -> NSView {
        if node.isLeaf {
            // リーフの全タブ content を渡す。非表示タブも subview として温存され、pty と
            // スクロールバックが死なない（model.panes が全タブを返すので contentCache も残る）。
            return PaneFrameView(
                node: node,
                coordinator: self,
                contents: node.panes.map { ($0.id, contentView(for: $0)) }
            )
        }
        let split = RatioSplitView()
        split.isVertical = (node.orientation == .horizontal)
        split.dividerStyle = .thin
        split.node = node
        split.delegate = split
        split.addArrangedSubview(buildNode(node.children[0]))
        split.addArrangedSubview(buildNode(node.children[1]))
        return split
    }

    private func contentView(for pane: PaneItem) -> NSView {
        if let cached = contentCache[pane.id] { return cached }
        let view: NSView
        switch pane.kind {
        case .terminal:
            let term = AppTerminalView(frame: NSRect(x: 0, y: 0, width: 480, height: 320))
            // ペイン専用 config で LABOLABO_PANE（ペイン UUID）をシェル環境へ注入する。
            // 手打ちの claude を含む子孫プロセスが hook 経由でタブと対応付くようになる。
            term.controller = TerminalController(
                configSource: Self.paneConfigSource(base: context.configSource, paneID: pane.id)
            )
            term.configuration = TerminalSurfaceOptions(
                backend: .exec,
                workingDirectory: context.workingDirectory
            )
            let delegate = TerminalLeafDelegate(pane: pane, coordinator: self)
            term.delegate = delegate
            terminalDelegates[pane.id] = delegate
            view = term
        case .files:
            view = NSHostingView(rootView: ChangedFilesPane(model: context.work))
        case .diff:
            view = NSHostingView(rootView: FileDetailPane(model: context.work))
        case .commits:
            view = NSHostingView(rootView: CommitGraphPane(model: context.work))
        }
        contentCache[pane.id] = view
        return view
    }

    // MARK: - ペイン専用 Ghostty config（LABOLABO_PANE の注入）

    /// ペイン専用の Ghostty config を生成する。ユーザー config を config-file で取り込みつつ、
    /// command を「/usr/bin/env LABOLABO_PANE=<ペインUUID> + ユーザーのログインシェル」へ
    /// 差し替える。これでこのペインのシェルと子孫プロセス（手打ちの claude を含む）が
    /// ペイン ID を環境変数として持ち、hook フォワーダが session_id とタブを対応付けられる。
    /// ユーザー config の `command` は意図的に無視する: LaboLabo のペインはコマンドを
    /// 打ち込む前提の「シェル」であることが必須で、ghostty 用の command 設定（tmux 等）を
    /// 持ち込むと ✨ 起動・自動 resume・WorkPane の前提が全部壊れるため。
    /// NOTE: libghostty の C API（ghostty_surface_config_s.env_vars）は env 注入を持つが、
    /// libghostty-spm 1.2.8 の TerminalSurfaceOptions が未公開のためこの方式を使う。
    /// upstream が対応したらこの関数ごと env 注入へ差し替える。
    static func paneConfigSource(
        base: TerminalController.ConfigSource, paneID: UUID
    ) -> TerminalController.ConfigSource {
        var lines: [String] = []
        switch base {
        case let .file(path):
            // ユーザー config は `config-file =` include ではなく**内容をそのまま**取り込む。
            // include は ghostty 1.3.1 CLI でも黙って失敗することを確認しており（実機では
            // フォント・テーマが既定に落ちた）、従来の .file 直読みと同一の入力を再現する
            // のが確実。読めない場合は素通し（libghostty 既定 + command だけになる）。
            // 注意: ユーザー config 内の相対パス config-file はこの方式では解決されない。
            if let contents = try? String(contentsOfFile: path, encoding: .utf8) {
                lines.append(contents)
            }
        case let .generated(contents):
            lines.append(contents)
        case .none:
            break
        }
        lines.append("command = /usr/bin/env LABOLABO_PANE=\(paneID.uuidString) \(loginShellCommand())")
        return .generated(lines.joined(separator: "\n") + "\n")
    }

    /// ユーザーのログインシェル（passwd → $SHELL → zsh の順）。`-l` でログインシェル動作を
    /// 再現する（.exec 既定のシェル起動に揃える）。
    private static func loginShellCommand() -> String {
        var shell = ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh"
        if let pw = getpwuid(getuid()), let cShell = pw.pointee.pw_shell, cShell.pointee != 0 {
            shell = String(cString: cShell)
        }
        return "\(shell) -l"
    }

    // MARK: actions invoked from AppKit headers / drops

    // NOTE: these only mutate the model. The rebuild is driven by SwiftUI via
    // `revision` → updateNSView → reconcile(), which runs on the *next* runloop
    // turn. Calling reconcile() synchronously here would tear down the very view
    // currently handling the drag / button tap (self) and crash AppKit.

    /// PaneFrameView（ヘッダーの分割ボタン）から呼ぶ。対象 pane は選択中タブとして解決済み。
    func split(_ paneID: UUID, _ orientation: NSUserInterfaceLayoutOrientation) {
        let newPane = PaneItem(kind: .terminal, title: String(localized: "端末"))
        model.split(paneID: paneID, orientation: orientation, newPane: newPane)
        // 開いた端末でそのまま入力できるように。
        pendingFocusPaneID = newPane.id
    }

    /// PaneFrameView（ヘッダーの閉じるボタン）から呼ぶ。閉じた結果前面になった端末へ
    /// フォーカスを渡す（ユーザー操作起点のときだけ。シェル exit 由来は handleProcessExit）。
    func close(_ paneID: UUID) {
        if let revealed = model.close(paneID: paneID),
           model.panes.first(where: { $0.id == revealed })?.kind == .terminal {
            pendingFocusPaneID = revealed
        }
    }

    func handleDrop(sourceID: UUID, targetID: UUID, edge: DropEdge) {
        // 実際に移動したときだけ、移動したタブ（端末なら）へ再構築後にフォーカスを移す。
        // no-op（同一グループ中央など）でセットすると、次の無関係な再構築で非表示タブへ
        // フォーカスが飛ぶので避ける。
        if model.move(sourceID, toEdgeOf: targetID, edge: edge),
           model.panes.first(where: { $0.id == sourceID })?.kind == .terminal {
            pendingFocusPaneID = sourceID
        }
    }

    /// 新規端末ペインのシェルが立ち上がった頃合いを見てテキスト（コマンド）を送る。
    /// 生成直後は AppTerminalView が未生成のことがあるので数回リトライする。
    func scheduleSend(paneID: UUID, text: String, attempt: Int = 0) {
        let delay = attempt == 0 ? 1.0 : 0.6
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self else { return }
            if let view = contentCache[paneID] as? AppTerminalView {
                view.sendText(text)
                // Enter(CR) は sendText だとテキスト扱いで実行されないため、Ghostty の
                // binding action `text:\r`（生バイト送信）で送って行を実行させる。
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.08) {
                    _ = view.performBindingAction("text:\\r")
                }
            } else if attempt < 6 {
                scheduleSend(paneID: paneID, text: text, attempt: attempt + 1)
            }
        }
    }
}

/// Captures title/close callbacks for one terminal leaf.
@MainActor
final class TerminalLeafDelegate: NSObject, TerminalSurfaceTitleDelegate, TerminalSurfaceCloseDelegate {
    weak var pane: PaneItem?
    weak var coordinator: TilingCoordinator?
    private let paneID: UUID

    init(pane: PaneItem, coordinator: TilingCoordinator) {
        self.pane = pane
        self.coordinator = coordinator
        paneID = pane.id
    }

    func terminalDidChangeTitle(_ title: String) {
        pane?.title = title.isEmpty ? String(localized: "端末") : title
    }

    func terminalDidClose(processAlive _: Bool) {
        coordinator?.handleProcessExit(paneID: paneID)
    }
}

extension TilingCoordinator {
    func handleProcessExit(paneID: UUID) {
        // Defer past the libghostty close callback; reconcile via `revision`.
        // グループに複数タブがあれば close はそのタブだけ閉じる（model.close 側で分岐）。
        model.close(paneID: paneID)
    }
}

// MARK: - Split view that remembers its ratio

final class RatioSplitView: NSSplitView, NSSplitViewDelegate {
    weak var node: TileNode?
    /// Guards against unbounded recursion: `setPosition` re-triggers `layout()`
    /// synchronously, which would call `applyRatio()` again, set the position
    /// again, … until the stack overflows. The flag makes the nested layout a
    /// no-op so the divider is positioned exactly once per layout pass.
    private var isApplyingRatio = false

    override func layout() {
        super.layout()
        applyRatio()
    }

    private func applyRatio() {
        guard !isApplyingRatio else { return }
        guard arrangedSubviews.count == 2 else { return }
        let dim = isVertical ? bounds.width : bounds.height
        guard dim > 0, let ratio = node?.ratio else { return }
        // AppKit が min/max 制約でクランプするのと同じ範囲に丸める。そうしないと
        // target が範囲外のとき current(クランプ後) と一致せず、毎レイアウトで
        // setPosition を再発行し続ける。
        let lo = paneMinSize
        let hi = max(lo, dim - dividerThickness - paneMinSize)
        let target = min(max((ratio * dim).rounded(), lo), hi)
        let current = isVertical ? arrangedSubviews[0].frame.width : arrangedSubviews[0].frame.height
        guard abs(current - target) > 1 else { return }
        isApplyingRatio = true
        setPosition(target, ofDividerAt: 0)
        isApplyingRatio = false
    }

    /// 各ペインに確保する最小サイズ。通常は 90pt だが、両側に 90pt×2 を取れないほど
    /// 狭いときは利用可能幅の半分まで縮める。これをしないと min 制約(+90)が max 制約(-90)を
    /// 追い越し（min>max）、AppKit のディバイダ配置が未定義動作になってレイアウトが崩れる
    /// （小さいウィンドウ・ネストした分割で発生）。
    private var paneMinSize: CGFloat {
        let dim = isVertical ? bounds.width : bounds.height
        let usable = dim - dividerThickness
        return max(0, min(90, usable / 2))
    }

    // NSSplitViewDelegate の min/max 制約は `ofSubviewAt`（`ofDividerAt` ではない）。
    // 誤ったラベルだとメソッドが呼ばれず、ペインを最小サイズ未満に潰せてしまう。
    func splitView(_ splitView: NSSplitView, constrainMinCoordinate proposedMin: CGFloat, ofSubviewAt _: Int) -> CGFloat {
        proposedMin + paneMinSize
    }

    func splitView(_ splitView: NSSplitView, constrainMaxCoordinate proposedMax: CGFloat, ofSubviewAt _: Int) -> CGFloat {
        proposedMax - paneMinSize
    }

    /// Persist the ratio only when the *user* drags a divider. `constrainSplitPosition`
    /// is called during interactive tracking (and by our own `setPosition`, which we
    /// ignore via `isApplyingRatio`). We deliberately do NOT update the ratio from
    /// `splitViewDidResizeSubviews`, because that fires for every transient layout
    /// pass during a rebuild and would clobber the stored ratio with a momentary
    /// equal-split value — the cause of panes resizing oddly after swap/move.
    func splitView(_ splitView: NSSplitView, constrainSplitPosition proposedPosition: CGFloat, ofSubviewAt _: Int) -> CGFloat {
        if !isApplyingRatio {
            let dim = isVertical ? bounds.width : bounds.height
            if dim > 0 {
                node?.ratio = max(0.05, min(0.95, proposedPosition / dim))
            }
        }
        return proposedPosition
    }
}

// MARK: - Leaf frame: header (SwiftUI) + tab contents + drop zones (AppKit)

struct PaneActions {
    var onSelect: (UUID) -> Void
    var splitRight: () -> Void
    var splitDown: () -> Void
    var close: () -> Void
    var canClose: Bool
}

final class PaneFrameView: NSView {
    private let node: TileNode
    private weak var coordinator: TilingCoordinator?
    /// self を参照する actions を張るため super.init 後に生成する（それまで nil）。
    private var header: NSView!
    /// タブ id → content view。選択中以外も subview として保持し（isHidden）、pty と
    /// スクロールバックを温存する。木を作り直さずタブ切替できるのがこの配列の役目。
    private let contents: [(id: UUID, view: NSView)]
    private let highlight = NSView()
    private static let headerHeight: CGFloat = 24

    init(node: TileNode, coordinator: TilingCoordinator, contents: [(UUID, NSView)]) {
        self.node = node
        self.coordinator = coordinator
        self.contents = contents.map { (id: $0.0, view: $0.1) }
        super.init(frame: .zero)

        wantsLayer = true
        // 全タブの content を重ねて配置（表示は showSelected の isHidden で 1 枚に絞る）。
        for entry in self.contents { addSubview(entry.view) }

        let hosting = NSHostingView(rootView: PaneHeader(node: node, actions: makeActions()))
        header = hosting
        addSubview(hosting)

        highlight.wantsLayer = true
        // ドロップ先ハイライトはブランド色（NSColor は非 Sendable のため inline 生成）。
        highlight.layer?.backgroundColor = NSColor(LaboTheme.brand).withAlphaComponent(0.28).cgColor
        highlight.isHidden = true
        addSubview(highlight)

        registerForDraggedTypes([.string])
        showSelected()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private func makeActions() -> PaneActions {
        PaneActions(
            onSelect: { [weak self] id in
                guard let self else { return }
                // 選択はモデルだけ更新（bump しない = reconcile なし）し、表示は自前の
                // isHidden 張り替えで即時反映。端末のチラつき防止のため木は作り直さない。
                self.coordinator?.model.selectTab(paneID: id)
                self.showSelected()
                self.focusSelectedTerminal()
            },
            splitRight: { [weak self] in self?.splitSelected(.horizontal) },
            splitDown: { [weak self] in self?.splitSelected(.vertical) },
            close: { [weak self] in self?.closeSelected() },
            canClose: (coordinator?.model.panes.count ?? 0) > 1
        )
    }

    // 分割 / 閉じるは「クリック時点の選択中タブ」を対象にする（生成時の固定 id ではない）。
    private func splitSelected(_ orientation: NSUserInterfaceLayoutOrientation) {
        guard let id = node.selectedPane?.id else { return }
        coordinator?.split(id, orientation)
    }

    private func closeSelected() {
        guard let id = node.selectedPane?.id else { return }
        coordinator?.close(id)
    }

    /// 選択中タブの content だけ表示。木を作り直さず isHidden を張り替えるだけなので
    /// 非表示タブの pty / スクロールバックは生きたまま。
    func showSelected() {
        let selectedID = node.selectedPane?.id
        for entry in contents {
            let hidden = (entry.id != selectedID)
            entry.view.isHidden = hidden
            // 非表示タブの ghostty サーフェスは描画（display link）を止めて電力を守る。
            // pty とスクロールバックは生きたままなので、再表示時に内容は最新に追いつく。
            (entry.view as? AppTerminalView)?.setSurfaceVisible(!hidden)
        }
    }

    /// タブ選択直後、表示された端末へキーフォーカスを移す（チップをクリックしたまま
    /// 打鍵できるように）。isHidden の反映後に行うため次のランループで実行する。
    private func focusSelectedTerminal() {
        guard let id = node.selectedPane?.id,
              let terminal = contents.first(where: { $0.id == id })?.view as? AppTerminalView else { return }
        DispatchQueue.main.async { [weak terminal] in
            guard let terminal, let window = terminal.window else { return }
            window.makeFirstResponder(terminal)
        }
    }

    override func layout() {
        super.layout()
        let h = Self.headerHeight
        // Non-flipped coordinates: header sits at the top (high y).
        header.frame = NSRect(x: 0, y: bounds.height - h, width: bounds.width, height: h)
        let contentRect = NSRect(x: 0, y: 0, width: bounds.width, height: max(0, bounds.height - h))
        // 全タブを同じ content rect に重ねる（表示は isHidden で 1 枚に絞る）。
        for entry in contents { entry.view.frame = contentRect }
    }

    // MARK: dragging destination

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation { update(sender) }
    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation { update(sender) }
    override func draggingExited(_: NSDraggingInfo?) { highlight.isHidden = true }
    override func draggingEnded(_: NSDraggingInfo) { highlight.isHidden = true }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        highlight.isHidden = true
        guard let string = sender.draggingPasteboard.string(forType: .string),
              let source = UUID(uuidString: string),
              let targetID = node.selectedPane?.id else { return false }
        let point = convert(sender.draggingLocation, from: nil)
        let dropEdge = edge(at: point)
        // 自リーフのタブ: 中央は合流済みで no-op、単独タブ（count==1）の端も分割の意味がなく no-op。
        if containsSource(source), dropEdge == .center || contents.count == 1 { return false }
        coordinator?.handleDrop(sourceID: source, targetID: targetID, edge: dropEdge)
        return true
    }

    private func update(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard let string = sender.draggingPasteboard.string(forType: .string),
              let source = UUID(uuidString: string) else {
            highlight.isHidden = true
            return []
        }
        let point = convert(sender.draggingLocation, from: nil)
        let dropEdge = edge(at: point)
        // 自リーフへのドロップは中央（合流済み）と単独タブの端を弾く。それ以外は許可。
        if containsSource(source), dropEdge == .center || contents.count == 1 {
            highlight.isHidden = true
            return []
        }
        highlight.frame = highlightRect(for: dropEdge)
        highlight.isHidden = false
        return .move
    }

    /// ドラッグ中の source がこのリーフのいずれかのタブか。
    private func containsSource(_ source: UUID) -> Bool {
        contents.contains { $0.id == source }
    }

    private func edge(at point: NSPoint) -> DropEdge {
        let w = bounds.width, h = bounds.height
        guard w > 0, h > 0 else { return .center }
        let rx = point.x / w, ry = point.y / h
        if rx < 0.25 { return .left }
        if rx > 0.75 { return .right }
        if ry > 0.75 { return .top }      // non-flipped: high y == top
        if ry < 0.25 { return .bottom }
        return .center
    }

    private func highlightRect(for edge: DropEdge) -> NSRect {
        let b = bounds
        switch edge {
        case .left: return NSRect(x: b.minX, y: b.minY, width: b.width / 2, height: b.height)
        case .right: return NSRect(x: b.midX, y: b.minY, width: b.width / 2, height: b.height)
        case .top: return NSRect(x: b.minX, y: b.midY, width: b.width, height: b.height / 2)
        case .bottom: return NSRect(x: b.minX, y: b.minY, width: b.width, height: b.height / 2)
        case .center: return b
        }
    }
}

/// SwiftUI header for one leaf: single title (1 tab) or a row of tab chips (2+).
struct PaneHeader: View {
    @Bindable var node: TileNode
    let actions: PaneActions

    var body: some View {
        row
            .buttonStyle(.borderless)
            .font(.system(size: 10))
            .padding(.horizontal, 6)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(LaboTheme.panel)
            // Web のヘッダ同様、下辺に 1px の罫線を敷いてコンテンツと区切る。
            .overlay(alignment: .bottom) { LaboTheme.border.frame(height: 1) }
            .contentShape(Rectangle())
            // 単一タブのときだけヘッダー全体がドラッグ源（そのペインを移動）。複数タブ時は
            // 各チップが個別に .onDrag するので、ヘッダー全体のドラッグは付けない。
            .modifier(PaneHeaderDrag(pane: node.panes.count == 1 ? node.selectedPane : nil))
    }

    @ViewBuilder private var row: some View {
        HStack(spacing: 6) {
            Image(systemName: "line.3.horizontal")
                .font(.system(size: 9))
                .foregroundStyle(.tertiary)
            if node.panes.count > 1 {
                ForEach(node.panes) { pane in
                    PaneTabChip(
                        pane: pane,
                        isSelected: pane.id == node.selectedPane?.id,
                        onSelect: { actions.onSelect(pane.id) }
                    )
                }
            } else {
                Image(systemName: node.selectedPane?.kind.iconName ?? "square")
                    .font(.system(size: 9))
                    .foregroundStyle(.secondary)
                Text(node.selectedPane?.title ?? "")
                    .font(.system(size: 11, weight: .medium))
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 4)
            Button(action: actions.splitRight) {
                Image(systemName: "rectangle.split.2x1")
            }
            .help("右に分割（新しい端末）")
            Button(action: actions.splitDown) {
                Image(systemName: "rectangle.split.1x2")
            }
            .help("下に分割（新しい端末）")
            if actions.canClose {
                Button(action: actions.close) {
                    Image(systemName: "xmark")
                }
                .help(node.panes.count > 1 ? "選択中のタブを閉じる" : "このペインを閉じる")
            }
        }
    }
}

/// タブバーの 1 チップ。タップで選択、ドラッグでそのタブだけを別ペインへ移動できる。
/// title は libghostty のタイトル変更で動的に変わる（PaneItem が @Observable なので追従）。
private struct PaneTabChip: View {
    @Bindable var pane: PaneItem
    let isSelected: Bool
    let onSelect: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: pane.kind.iconName)
                .font(.system(size: 9))
            Text(pane.title)
                .lineLimit(1)
                .truncationMode(.middle)
        }
        .font(.system(size: 11, weight: .medium))
        .foregroundStyle(isSelected ? .primary : .secondary)
        .padding(.horizontal, 6)
        .padding(.vertical, 2)
        .background(isSelected ? LaboTheme.panelRaised : Color.clear)
        .clipShape(RoundedRectangle(cornerRadius: 4))
        .contentShape(Rectangle())
        .onTapGesture { onSelect() }
        .onDrag { NSItemProvider(object: pane.id.uuidString as NSString) }
        .help(pane.title)
    }
}

/// 単一タブのヘッダーだけ、全体をドラッグ源にする条件付きモディファイア。
private struct PaneHeaderDrag: ViewModifier {
    let pane: PaneItem?

    func body(content: Content) -> some View {
        if let pane {
            content.onDrag { NSItemProvider(object: pane.id.uuidString as NSString) }
        } else {
            content
        }
    }
}
