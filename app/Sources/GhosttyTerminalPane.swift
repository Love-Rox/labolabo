import SwiftUI
import GhosttyTerminal

/// A single libghostty-backed terminal surface running a login shell in
/// `workingDirectory`. One `TerminalViewState` owns one surface; keep the pane
/// mounted (e.g. via opacity, not conditional removal) to keep the surface alive
/// across tab switches.
struct GhosttyTerminalPane: View {
    @StateObject private var state: TerminalViewState

    init(workingDirectory: String) {
        let state = TerminalViewState(terminalConfiguration: .default)
        state.configuration = TerminalSurfaceOptions(
            backend: .exec,
            workingDirectory: workingDirectory
        )
        _state = StateObject(wrappedValue: state)
    }

    var body: some View {
        TerminalSurfaceView(context: state)
            .background(Color.black)
    }
}
