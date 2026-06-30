import SwiftUI
import GhosttyTerminal

/// A single libghostty-backed terminal surface. The `TerminalViewState` (which
/// owns the surface/pty) is created and held by the model (`TerminalLeaf`) so the
/// surface survives SwiftUI view-tree reshuffles (tab switch / split / swap).
/// Keep the pane mounted (e.g. via opacity, not conditional removal) to keep the
/// surface alive across tab switches.
struct GhosttyTerminalPane: View {
    let state: TerminalViewState

    var body: some View {
        TerminalSurfaceView(context: state)
            .background(Color.black)
    }
}
