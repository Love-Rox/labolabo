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

    init(root: TileNode) { self.root = root }

    static func defaultLayout() -> PaneTilingModel {
        // Terminal on top; bottom row = changed-files (left) | diff (right).
        let terminal = TileNode(pane: PaneItem(kind: .terminal, title: "端末"))
        let files = TileNode(pane: PaneItem(kind: .files, title: "変更ファイル"))
        let diff = TileNode(pane: PaneItem(kind: .diff, title: "Diff"))
        let bottom = TileNode(orientation: .horizontal, ratio: 0.42, children: [files, diff])
        let root = TileNode(orientation: .vertical, ratio: 0.58, children: [terminal, bottom])
        return PaneTilingModel(root: root)
    }

    private func bump() { revision &+= 1 }

    var panes: [PaneItem] { Self.leaves(root).compactMap(\.pane) }
    func hasPane(kind: PaneKind) -> Bool { panes.contains { $0.kind == kind } }

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
        TilingCoordinator(model: model, context: context)
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

    init(model: PaneTilingModel, context: PaneContext) {
        self.model = model
        self.context = context
    }

    func reconcile() {
        guard let container else { return }
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

    override func layout() {
        super.layout()
        applyRatio()
    }

    private func applyRatio() {
        guard arrangedSubviews.count == 2 else { return }
        let dim = isVertical ? bounds.width : bounds.height
        guard dim > 0, let ratio = node?.ratio else { return }
        let target = ratio * dim
        let current = isVertical ? arrangedSubviews[0].frame.width : arrangedSubviews[0].frame.height
        if abs(current - target) > 0.5 {
            setPosition(target, ofDividerAt: 0)
        }
    }

    func splitView(_ splitView: NSSplitView, constrainMinCoordinate proposedMin: CGFloat, ofDividerAt _: Int) -> CGFloat {
        proposedMin + 90
    }

    func splitView(_ splitView: NSSplitView, constrainMaxCoordinate proposedMax: CGFloat, ofDividerAt _: Int) -> CGFloat {
        proposedMax - 90
    }

    func splitViewDidResizeSubviews(_: Notification) {
        guard arrangedSubviews.count == 2 else { return }
        let dim = isVertical ? bounds.width : bounds.height
        guard dim > 0 else { return }
        let size = isVertical ? arrangedSubviews[0].frame.width : arrangedSubviews[0].frame.height
        node?.ratio = max(0.05, min(0.95, size / dim))
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
        }
    }
}

/// Slim toolbar to add a terminal or re-add the files/diff panes after closing.
struct PaneToolbar: View {
    @Bindable var model: PaneTilingModel

    var body: some View {
        HStack(spacing: 10) {
            Button { model.addPane(PaneItem(kind: .terminal, title: "端末")) } label: {
                Label("端末", systemImage: "plus.rectangle")
            }
            Button { model.addPaneIfAbsent(kind: .files, title: "変更ファイル") } label: {
                Label("ファイル", systemImage: "list.bullet.rectangle")
            }
            .disabled(model.hasPane(kind: .files))
            Button { model.addPaneIfAbsent(kind: .diff, title: "Diff") } label: {
                Label("Diff", systemImage: "doc.text")
            }
            .disabled(model.hasPane(kind: .diff))
            Spacer()
            Text("ヘッダーをドラッグ→他ペインの縁で分割 / 中央で入れ替え")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .buttonStyle(.borderless)
        .font(.caption)
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
    }
}
