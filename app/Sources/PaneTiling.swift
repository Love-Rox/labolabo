import SwiftUI
import AppKit
import Observation
import GhosttyTerminal

// MARK: - AppKit / SwiftUI 層（モデルは PaneTilingModel.swift）
//
// タイル/タブ木の *モデル* は UI 非依存の `PaneTilingModel.swift` に分離した。本ファイルは
// その木を AppKit（NSSplitView / NSView）と SwiftUI（NSViewRepresentable・ヘッダー）へ橋渡し
// する UI 層。TilingCoordinator が葉の NSView を所有し、木の変化に応じて NSSplitView 間で
// *reparent* するだけなので、端末の pty / スクロールバックが split / move / タブ合流をまたいで
// 生き残る（AppTerminalView が reparent 後もサーフェスを保持する）。別リーフの中央へドロップ
// すればタブとして合流し、端へドロップすれば分割する。
//
// モデルとの結合点は 2 つだけ:
//   - 向き: モデルの `TileOrientation` と AppKit の `NSUserInterfaceLayoutOrientation` を
//     下の extension で相互変換する。
//   - 操作面: `TilingCoordinator` が `PaneTilingActions`（端末へのテキスト送信・フォーカス
//     指定）に準拠し、モデルからの依頼を受ける。

// MARK: - AppKit ↔ モデルの向き変換

extension TileOrientation {
    /// AppKit の向きからモデルの向きへ。
    init(_ ns: NSUserInterfaceLayoutOrientation) {
        self = (ns == .vertical) ? .vertical : .horizontal
    }

    /// モデルの向きから AppKit の向きへ。
    var nsOrientation: NSUserInterfaceLayoutOrientation {
        self == .vertical ? .vertical : .horizontal
    }
}

/// Inputs the tiling needs to build leaf content.
struct PaneContext {
    let workingDirectory: String
    let work: WorkPaneModel
    let configSource: TerminalController.ConfigSource
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
final class TilingCoordinator: NSObject, PaneTilingActions {
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
        split.isVertical = (node.orientation.nsOrientation == .horizontal)
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
        model.split(paneID: paneID, orientation: TileOrientation(orientation), newPane: newPane)
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

    /// `PaneTilingActions` の witness。attempt=0 からリトライ送信を開始する。
    func scheduleSend(paneID: UUID, text: String) {
        scheduleSend(paneID: paneID, text: text, attempt: 0)
    }

    /// 新規端末ペインのシェルが立ち上がった頃合いを見てテキスト（コマンド）を送る。
    /// 生成直後は AppTerminalView が未生成のことがあるので数回リトライする。
    func scheduleSend(paneID: UUID, text: String, attempt: Int) {
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
