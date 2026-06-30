import SwiftUI
import Observation
import GhosttyTerminal

// MARK: - Model
//
// 階層は worktree → tab → pane(binary split tree) → surface。
// 端末の surface を保持する `TerminalViewState` はモデル側（`TerminalLeaf`）が
// 所有し、タブ切替・分割・入れ替えで SwiftUI のビュー木が組み替わっても pty が
// 生き続けるようにする（ビューは状態を生成せず参照するだけ）。

/// 1 つの libghostty surface（= 1 端末）。
@MainActor
final class TerminalLeaf: Identifiable {
    let id = UUID()
    var title: String
    let state: TerminalViewState

    init(workingDirectory: String, title: String = "shell") {
        self.title = title
        let state = TerminalViewState(terminalConfiguration: .default)
        state.configuration = TerminalSurfaceOptions(
            backend: .exec,
            workingDirectory: workingDirectory
        )
        self.state = state
    }
}

/// タブ内の分割ツリーのノード。`terminal != nil` なら葉（1 端末）、そうでなければ
/// `axis` 方向に 2 つの子を持つ分割。
@MainActor
@Observable
final class PaneNode: Identifiable {
    let id = UUID()
    var terminal: TerminalLeaf?
    var axis: Axis = .horizontal
    var children: [PaneNode] = []

    init(terminal: TerminalLeaf) { self.terminal = terminal }
    init(axis: Axis, children: [PaneNode]) {
        self.axis = axis
        self.children = children
    }

    var isLeaf: Bool { terminal != nil }
}

/// 1 タブ = 1 分割ツリー + フォーカス中の葉（分割や閉じる操作の対象）。
@MainActor
@Observable
final class TerminalTab: Identifiable {
    let id = UUID()
    let root: PaneNode
    var focusedLeafID: UUID

    init(workingDirectory: String) {
        let leaf = TerminalLeaf(workingDirectory: workingDirectory)
        self.root = PaneNode(terminal: leaf)
        self.focusedLeafID = leaf.id
    }

    var leafCount: Int { Self.leaves(in: root).count }

    static func leaves(in node: PaneNode) -> [TerminalLeaf] {
        if let terminal = node.terminal { return [terminal] }
        return node.children.flatMap { leaves(in: $0) }
    }
}

@MainActor
@Observable
final class TerminalAreaModel {
    let workingDirectory: String
    var tabs: [TerminalTab] = []
    var selectedTabID: UUID

    init(workingDirectory: String) {
        self.workingDirectory = workingDirectory
        let tab = TerminalTab(workingDirectory: workingDirectory)
        self.tabs = [tab]
        self.selectedTabID = tab.id
    }

    var selectedTab: TerminalTab? { tabs.first { $0.id == selectedTabID } }

    func addTab() {
        let tab = TerminalTab(workingDirectory: workingDirectory)
        tabs.append(tab)
        selectedTabID = tab.id
    }

    func closeTab(_ id: UUID) {
        tabs.removeAll { $0.id == id }
        if tabs.isEmpty {
            addTab()
        } else if selectedTabID == id {
            selectedTabID = tabs.last!.id
        }
    }

    /// 選択タブのフォーカス中ペインを 2 分割し、既存端末の隣に新しい端末を置く。
    /// 既存 surface はモデルが保持し続けるので、そのまま動き続ける。
    func splitFocused(_ axis: Axis) {
        guard let tab = selectedTab,
              let node = Self.findLeafNode(tab.root, leafID: tab.focusedLeafID),
              let terminal = node.terminal else { return }
        let newLeaf = TerminalLeaf(workingDirectory: workingDirectory)
        let keep = PaneNode(terminal: terminal)
        let added = PaneNode(terminal: newLeaf)
        node.terminal = nil
        node.axis = axis
        node.children = [keep, added]
        tab.focusedLeafID = newLeaf.id
    }

    /// 指定ペインを閉じ、親の分割を残る兄弟に畳む。タブ唯一のペインならタブごと閉じる。
    func closePane(leafID: UUID, in tab: TerminalTab) {
        guard let (parent, index) = Self.findParent(tab.root, leafID: leafID) else {
            closeTab(tab.id)   // フォーカス葉が root = タブ最後のペイン
            return
        }
        let sibling = parent.children[index == 0 ? 1 : 0]
        parent.terminal = sibling.terminal
        parent.axis = sibling.axis
        parent.children = sibling.children
        if Self.findLeafNode(tab.root, leafID: tab.focusedLeafID) == nil {
            tab.focusedLeafID = Self.firstLeaf(parent).id
        }
    }

    /// 2 つのペインの位置を入れ替える（端末参照だけを交換 → surface は生かしたまま）。
    func swapPanes(_ a: UUID, _ b: UUID, in tab: TerminalTab) {
        guard a != b,
              let nodeA = Self.findLeafNode(tab.root, leafID: a),
              let nodeB = Self.findLeafNode(tab.root, leafID: b) else { return }
        let tmp = nodeA.terminal
        nodeA.terminal = nodeB.terminal
        nodeB.terminal = tmp
    }

    // MARK: tree helpers

    static func findLeafNode(_ node: PaneNode, leafID: UUID) -> PaneNode? {
        if node.terminal?.id == leafID { return node }
        for child in node.children {
            if let found = findLeafNode(child, leafID: leafID) { return found }
        }
        return nil
    }

    static func findParent(_ node: PaneNode, leafID: UUID) -> (PaneNode, Int)? {
        for (index, child) in node.children.enumerated() {
            if child.terminal?.id == leafID { return (node, index) }
            if let found = findParent(child, leafID: leafID) { return found }
        }
        return nil
    }

    static func firstLeaf(_ node: PaneNode) -> TerminalLeaf {
        if let terminal = node.terminal { return terminal }
        return firstLeaf(node.children[0])
    }
}

// MARK: - Views

/// 複数 libghostty 端末をタブで持ち、各タブをペイン分割できる領域。
/// 非アクティブタブも opacity で隠して mount したままにし、surface を生かす。
struct TerminalAreaView: View {
    @State private var model: TerminalAreaModel

    init(workingDirectory: String) {
        _model = State(initialValue: TerminalAreaModel(workingDirectory: workingDirectory))
    }

    var body: some View {
        VStack(spacing: 0) {
            tabBar
            Divider()
            ZStack {
                Color.black
                ForEach(model.tabs) { tab in
                    PaneTreeView(model: model, tab: tab, node: tab.root,
                                 showsChrome: tab.leafCount > 1)
                        .opacity(tab.id == model.selectedTabID ? 1 : 0)
                        .allowsHitTesting(tab.id == model.selectedTabID)
                }
            }
        }
    }

    private var tabBar: some View {
        HStack(spacing: 4) {
            ForEach(Array(model.tabs.enumerated()), id: \.element.id) { index, tab in
                tabChip(tab, index: index)
            }
            Button { model.addTab() } label: {
                Image(systemName: "plus")
            }
            .buttonStyle(.borderless)
            .help("新しい端末タブ")

            Spacer()

            Button { model.splitFocused(.horizontal) } label: {
                Image(systemName: "rectangle.split.2x1")
            }
            .buttonStyle(.borderless)
            .help("フォーカス中のペインを左右に分割")

            Button { model.splitFocused(.vertical) } label: {
                Image(systemName: "rectangle.split.1x2")
            }
            .buttonStyle(.borderless)
            .help("フォーカス中のペインを上下に分割")
        }
        .padding(6)
    }

    private func tabChip(_ tab: TerminalTab, index: Int) -> some View {
        let isSelected = tab.id == model.selectedTabID
        return HStack(spacing: 4) {
            Text("端末 \(index + 1)").font(.caption)
            if model.tabs.count > 1 {
                Button { model.closeTab(tab.id) } label: {
                    Image(systemName: "xmark").font(.system(size: 8, weight: .bold))
                }
                .buttonStyle(.borderless)
                .help("タブを閉じる")
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .background(
            isSelected ? Color.accentColor.opacity(0.25) : Color.clear,
            in: RoundedRectangle(cornerRadius: 5)
        )
        .contentShape(Rectangle())
        .onTapGesture { model.selectedTabID = tab.id }
    }
}

/// 分割ツリーを再帰描画。葉は端末、分割は H/V SplitView。
struct PaneTreeView: View {
    let model: TerminalAreaModel
    let tab: TerminalTab
    let node: PaneNode
    let showsChrome: Bool

    var body: some View {
        if let leaf = node.terminal {
            PaneLeafView(model: model, tab: tab, leaf: leaf, showsChrome: showsChrome)
                .id(leaf.id)
        } else if node.axis == .horizontal {
            HSplitView {
                PaneTreeView(model: model, tab: tab, node: node.children[0], showsChrome: showsChrome)
                PaneTreeView(model: model, tab: tab, node: node.children[1], showsChrome: showsChrome)
            }
        } else {
            VSplitView {
                PaneTreeView(model: model, tab: tab, node: node.children[0], showsChrome: showsChrome)
                PaneTreeView(model: model, tab: tab, node: node.children[1], showsChrome: showsChrome)
            }
        }
    }
}

/// 1 ペイン（端末 + 分割時のヘッダー）。ヘッダーをドラッグ→他ペインにドロップで入れ替え。
struct PaneLeafView: View {
    let model: TerminalAreaModel
    let tab: TerminalTab
    let leaf: TerminalLeaf
    let showsChrome: Bool

    @State private var isDropTargeted = false

    private var isFocused: Bool { tab.focusedLeafID == leaf.id }

    var body: some View {
        VStack(spacing: 0) {
            if showsChrome {
                paneHeader
                Divider()
            }
            GhosttyTerminalPane(state: leaf.state)
        }
        .frame(minWidth: 140, minHeight: 90)
        .overlay(borderOverlay)
        .dropDestination(for: String.self) { items, _ in
            guard showsChrome, let source = items.first,
                  let sourceID = UUID(uuidString: source) else { return false }
            model.swapPanes(sourceID, leaf.id, in: tab)
            return true
        } isTargeted: { targeted in
            isDropTargeted = targeted
        }
    }

    private var borderOverlay: some View {
        Rectangle()
            .strokeBorder(borderColor, lineWidth: isDropTargeted ? 2.5 : 1.5)
            .allowsHitTesting(false)
    }

    private var borderColor: Color {
        if isDropTargeted { return .accentColor }
        if showsChrome && isFocused { return .accentColor }
        return .clear
    }

    private var paneHeader: some View {
        HStack(spacing: 4) {
            Image(systemName: "line.3.horizontal")
                .font(.system(size: 9))
                .foregroundStyle(.secondary)
            Circle()
                .fill(isFocused ? Color.accentColor : Color.secondary.opacity(0.5))
                .frame(width: 6, height: 6)
            Text(leaf.title).font(.caption2)
            Spacer()
            Button {
                model.closePane(leafID: leaf.id, in: tab)
            } label: {
                Image(systemName: "xmark").font(.system(size: 8, weight: .bold))
            }
            .buttonStyle(.borderless)
            .help("ペインを閉じる")
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 3)
        .background(isFocused ? Color.accentColor.opacity(0.15) : Color.black.opacity(0.001))
        .contentShape(Rectangle())
        .onTapGesture { tab.focusedLeafID = leaf.id }
        .draggable(leaf.id.uuidString) {
            // ドラッグ中のプレビュー
            Label(leaf.title, systemImage: "macwindow")
                .padding(6)
                .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 6))
        }
    }
}
