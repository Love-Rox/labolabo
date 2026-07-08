import Foundation

/// 同時実行数を `limit` までに抑える非ブロッキングのゲート。
///
/// 空きがなければ continuation を積んで suspend するだけで、スレッドは
/// 一切ブロックしない。解放時は先頭の待機者へスロットをそのまま譲る（FIFO）。
actor ConcurrencyGate {
    private let limit: Int
    private var running = 0
    private var waiters: [CheckedContinuation<Void, Never>] = []

    init(limit: Int) {
        self.limit = limit
    }

    func acquire() async {
        if running < limit {
            running += 1
            return
        }
        await withCheckedContinuation { waiters.append($0) }
    }

    func release() {
        if waiters.isEmpty {
            running -= 1
        } else {
            waiters.removeFirst().resume()
        }
    }
}
