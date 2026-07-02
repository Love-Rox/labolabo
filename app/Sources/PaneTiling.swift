import SwiftUI
import AppKit
import Observation
import GhosttyTerminal

// MARK: - Model
//
// One session's work area is a single binary tile tree whose leaves are any of:
// a terminal surface, the changed-files pane, or the diff pane. The AppKit layer
// (TilingCoordinator) owns the leaf NSViews and only *reparents* them between
// NSSplitViews as the tree changes, so a terminal's pty/scrollback survives
// split / move / swap (AppTerminalView keeps its surface across reparenting).

enum PaneKind: String {
    case terminal
    case files
    case diff
    case commits

    /// 復元時にタイトルが欠けていた場合の既定タイトル。
    var defaultTitle: String {
        switch self {
        case .terminal: return "端末"
        case .files: return "変更ファイル"
        case .diff: return "Diff"
        case .commits: return "履歴"
        }
    }
}

/// タイル配置の保存表現（Codable）。leaf は `paneKind`/`paneTitle`、split は
/// `orientation`/`ratio`/`children`。セッション別配置と名前付きプリセットに使う。
struct TileLayout: Codable, Equatable {
    var paneKind: String?
    var paneTitle: String?
    var orientation: String?
    var ratio: CGFloat?
    var children: [TileLayout]?

    init(
        paneKind: String? = nil, paneTitle: String? = nil,
        orientation: String? = nil, ratio: CGFloat? = nil, children: [TileLayout]? = nil
    ) {
        self.paneKind = paneKind
        self.paneTitle = paneTitle
        self.orientation = orientation
        self.ratio = ratio
        self.children = children
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

    init(kind: PaneKind, title: String) {
        self.kind = kind
        self.title = title
    }
}

/// A node in the binary tile tree: a leaf (`pane != nil`) or a 2-way split.
@MainActor
@Observable
final class TileNode: Identifiable {
    let id = UUID()
    var pane: PaneItem?
    var orientation: NSUserInterfaceLayoutOrientation
    var children: [TileNode]
    /// First child's fraction of the split (0…1). Persisted across rebuilds.
    var ratio: CGFloat

    init(pane: PaneItem) {
        self.pane = pane
        orientation = .horizontal
        children = []
        ratio = 0.5
    }

    init(orientation: NSUserInterfaceLayoutOrientation, ratio: CGFloat, children: [TileNode]) {
        pane = nil
        self.orientation = orientation
        self.ratio = ratio
        self.children = children
    }

    var isLeaf: Bool { pane != nil }
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
    /// 構造変更（追加/削除/分割/移動/入替）ごとに呼ばれる。配置の永続化に使う。
    /// ratio（ドラッグ）は bump しないので、離脱時に別途 snapshot して保存する。
    @ObservationIgnored var onLayoutChanged: (() -> Void)?

    init(root: TileNode) { self.root = root }

    /// 新しい端末ペインを作り、シェル起動後にコマンドを送る（Claude 起動などに使用）。
    func launchInNewTerminal(title: String, command: String) {
        let pane = PaneItem(kind: .terminal, title: title)
        addPane(pane)
        coordinator?.scheduleSend(paneID: pane.id, text: command)
    }

    static func defaultLayout() -> PaneTilingModel {
        // Terminal on top; bottom row = commit graph | changed-files | diff (1:1:2).
        let terminal = TileNode(pane: PaneItem(kind: .terminal, title: "端末"))
        let commits = TileNode(pane: PaneItem(kind: .commits, title: "履歴"))
        let files = TileNode(pane: PaneItem(kind: .files, title: "変更ファイル"))
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

    var panes: [PaneItem] { Self.leaves(root).compactMap(\.pane) }
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
        if let pane = node.pane {
            return TileLayout(paneKind: pane.kind.rawValue, paneTitle: pane.title)
        }
        return TileLayout(
            orientation: node.orientation == .vertical ? "vertical" : "horizontal",
            ratio: node.ratio,
            children: node.children.map(encode)
        )
    }

    private static func decode(_ layout: TileLayout) -> TileNode? {
        if let kindRaw = layout.paneKind, let kind = PaneKind(rawValue: kindRaw) {
            return TileNode(pane: PaneItem(kind: kind, title: layout.paneTitle ?? kind.defaultTitle))
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
        guard let node = Self.findLeaf(root, paneID: paneID), let existing = node.pane else { return }
        let keep = TileNode(pane: existing)
        let added = TileNode(pane: newPane)
        node.pane = nil
        node.orientation = orientation
        node.children = [keep, added]
        node.ratio = 0.5
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

    func close(paneID: UUID) {
        guard let node = Self.findLeaf(root, paneID: paneID),
              let (parent, index) = Self.findParent(root, childID: node.id) else {
            return // root-only pane: keep at least one
        }
        collapse(parent, into: parent.children[index == 0 ? 1 : 0])
        bump()
    }

    func move(_ sourceID: UUID, toEdgeOf targetID: UUID, edge: DropEdge) {
        guard sourceID != targetID else { return }
        if edge == .center { swap(sourceID, targetID); return }
        guard let sourceNode = Self.findLeaf(root, paneID: sourceID),
              let sourcePane = sourceNode.pane else { return }
        detach(sourceNode)
        // Tree changed; re-find the target by pane id.
        guard let targetNode = Self.findLeaf(root, paneID: targetID),
              let targetPane = targetNode.pane else { bump(); return }

        let orientation: NSUserInterfaceLayoutOrientation =
            (edge == .left || edge == .right) ? .horizontal : .vertical
        let sourceOnSecond = (edge == .right || edge == .bottom)
        let keep = TileNode(pane: targetPane)
        let moved = TileNode(pane: sourcePane)
        targetNode.pane = nil
        targetNode.orientation = orientation
        targetNode.children = sourceOnSecond ? [keep, moved] : [moved, keep]
        targetNode.ratio = 0.5
        bump()
    }

    func swap(_ a: UUID, _ b: UUID) {
        guard a != b,
              let na = Self.findLeaf(root, paneID: a),
              let nb = Self.findLeaf(root, paneID: b) else { return }
        let tmp = na.pane
        na.pane = nb.pane
        nb.pane = tmp
        bump()
    }

    /// Remove a leaf by collapsing its parent into the sibling (no bump).
    private func detach(_ node: TileNode) {
        guard let (parent, index) = Self.findParent(root, childID: node.id) else { return }
        collapse(parent, into: parent.children[index == 0 ? 1 : 0])
    }

    private func collapse(_ parent: TileNode, into sibling: TileNode) {
        parent.pane = sibling.pane
        parent.orientation = sibling.orientation
        parent.children = sibling.children
        parent.ratio = sibling.ratio
    }

    // MARK: tree helpers

    static func leaves(_ node: TileNode) -> [TileNode] {
        node.isLeaf ? [node] : node.children.flatMap(leaves)
    }

    static func findLeaf(_ node: TileNode, paneID: UUID) -> TileNode? {
        if node.pane?.id == paneID { return node }
        for child in node.children {
            if let found = findLeaf(child, paneID: paneID) { return found }
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
        // パルス・git 更新など）で updateNSView が走ってもツリーを壊さず、チラつきを防ぐ。
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

        // Release content (and surfaces) for panes that no longer exist.
        for id in contentCache.keys where !liveIDs.contains(id) {
            contentCache[id] = nil
            terminalDelegates[id] = nil
        }
    }

    private func buildNode(_ node: TileNode) -> NSView {
        if let pane = node.pane {
            return PaneFrameView(
                pane: pane,
                coordinator: self,
                content: contentView(for: pane)
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
            term.controller = TerminalController(configSource: context.configSource)
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

    // MARK: actions invoked from AppKit headers / drops

    func actions(for pane: PaneItem) -> PaneActions {
        PaneActions(
            splitRight: { [weak self] in self?.split(pane.id, .horizontal) },
            splitDown: { [weak self] in self?.split(pane.id, .vertical) },
            close: { [weak self] in self?.close(pane.id) },
            canClose: model.panes.count > 1
        )
    }

    // NOTE: these only mutate the model. The rebuild is driven by SwiftUI via
    // `revision` → updateNSView → reconcile(), which runs on the *next* runloop
    // turn. Calling reconcile() synchronously here would tear down the very view
    // currently handling the drag / button tap (self) and crash AppKit.

    private func split(_ paneID: UUID, _ orientation: NSUserInterfaceLayoutOrientation) {
        model.split(paneID: paneID, orientation: orientation, newPane: PaneItem(kind: .terminal, title: "端末"))
    }

    private func close(_ paneID: UUID) {
        model.close(paneID: paneID)
    }

    func handleDrop(sourceID: UUID, targetID: UUID, edge: DropEdge) {
        model.move(sourceID, toEdgeOf: targetID, edge: edge)
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
        pane?.title = title.isEmpty ? "端末" : title
    }

    func terminalDidClose(processAlive _: Bool) {
        coordinator?.handleProcessExit(paneID: paneID)
    }
}

extension TilingCoordinator {
    func handleProcessExit(paneID: UUID) {
        // Defer past the libghostty close callback; reconcile via `revision`.
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
        let target = (ratio * dim).rounded()
        let current = isVertical ? arrangedSubviews[0].frame.width : arrangedSubviews[0].frame.height
        guard abs(current - target) > 1 else { return }
        isApplyingRatio = true
        setPosition(target, ofDividerAt: 0)
        isApplyingRatio = false
    }

    // NSSplitViewDelegate の min/max 制約は `ofSubviewAt`（`ofDividerAt` ではない）。
    // 誤ったラベルだとメソッドが呼ばれず、ペインを 90pt 未満に潰せてしまう。
    func splitView(_ splitView: NSSplitView, constrainMinCoordinate proposedMin: CGFloat, ofSubviewAt _: Int) -> CGFloat {
        proposedMin + 90
    }

    func splitView(_ splitView: NSSplitView, constrainMaxCoordinate proposedMax: CGFloat, ofSubviewAt _: Int) -> CGFloat {
        proposedMax - 90
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

// MARK: - Leaf frame: header (SwiftUI) + content + drop zones (AppKit)

struct PaneActions {
    var splitRight: () -> Void
    var splitDown: () -> Void
    var close: () -> Void
    var canClose: Bool
}

final class PaneFrameView: NSView {
    private let paneID: UUID
    private weak var coordinator: TilingCoordinator?
    private let header: NSView
    private let content: NSView
    private let highlight = NSView()
    private static let headerHeight: CGFloat = 24

    init(pane: PaneItem, coordinator: TilingCoordinator, content: NSView) {
        paneID = pane.id
        self.coordinator = coordinator
        self.content = content
        header = NSHostingView(
            rootView: PaneHeader(pane: pane, actions: coordinator.actions(for: pane))
        )
        super.init(frame: .zero)

        wantsLayer = true
        addSubview(content)
        addSubview(header)

        highlight.wantsLayer = true
        highlight.layer?.backgroundColor = NSColor.controlAccentColor.withAlphaComponent(0.28).cgColor
        highlight.isHidden = true
        addSubview(highlight)

        registerForDraggedTypes([.string])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func layout() {
        super.layout()
        let h = Self.headerHeight
        // Non-flipped coordinates: header sits at the top (high y).
        header.frame = NSRect(x: 0, y: bounds.height - h, width: bounds.width, height: h)
        content.frame = NSRect(x: 0, y: 0, width: bounds.width, height: max(0, bounds.height - h))
    }

    // MARK: dragging destination

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation { update(sender) }
    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation { update(sender) }
    override func draggingExited(_: NSDraggingInfo?) { highlight.isHidden = true }
    override func draggingEnded(_: NSDraggingInfo) { highlight.isHidden = true }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        highlight.isHidden = true
        guard let string = sender.draggingPasteboard.string(forType: .string),
              let source = UUID(uuidString: string), source != paneID else { return false }
        let point = convert(sender.draggingLocation, from: nil)
        coordinator?.handleDrop(sourceID: source, targetID: paneID, edge: edge(at: point))
        return true
    }

    private func update(_ sender: NSDraggingInfo) -> NSDragOperation {
        if let string = sender.draggingPasteboard.string(forType: .string), string == paneID.uuidString {
            highlight.isHidden = true
            return []
        }
        let point = convert(sender.draggingLocation, from: nil)
        highlight.frame = highlightRect(for: edge(at: point))
        highlight.isHidden = false
        return .move
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

/// SwiftUI header for one pane: title + split/close + drag handle.
struct PaneHeader: View {
    @Bindable var pane: PaneItem
    let actions: PaneActions

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "line.3.horizontal")
                .font(.system(size: 9))
                .foregroundStyle(.tertiary)
            Image(systemName: icon)
                .font(.system(size: 9))
                .foregroundStyle(.secondary)
            Text(pane.title)
                .font(.system(size: 11))
                .lineLimit(1)
                .truncationMode(.middle)
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
                .help("このペインを閉じる")
            }
        }
        .buttonStyle(.borderless)
        .font(.system(size: 10))
        .padding(.horizontal, 6)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .windowBackgroundColor))
        .contentShape(Rectangle())
        .onDrag { NSItemProvider(object: pane.id.uuidString as NSString) }
    }

    private var icon: String {
        switch pane.kind {
        case .terminal: return "terminal"
        case .files: return "list.bullet.rectangle"
        case .diff: return "doc.text"
        case .commits: return "point.3.connected.trianglepath.dotted"
        }
    }
}
