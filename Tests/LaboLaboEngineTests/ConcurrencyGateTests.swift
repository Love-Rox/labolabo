import XCTest
@testable import LaboLaboEngine

final class ConcurrencyGateTests: XCTestCase {

    /// 同時実行数のピークを lock 越しに記録するカウンタ。
    private final class PeakCounter: @unchecked Sendable {
        private let lock = NSLock()
        private var current = 0
        private(set) var peak = 0

        func enter() {
            lock.lock()
            current += 1
            peak = max(peak, current)
            lock.unlock()
        }

        func leave() {
            lock.lock()
            current -= 1
            lock.unlock()
        }
    }

    func testConcurrencyNeverExceedsLimitAndAllComplete() async {
        let gate = ConcurrencyGate(limit: 3)
        let counter = PeakCounter()

        let completed = await withTaskGroup(of: Void.self, returning: Int.self) { group in
            for _ in 0 ..< 30 {
                group.addTask {
                    await gate.acquire()
                    counter.enter()
                    // 区間の重なりを作って peak を意味のある値にする。
                    for _ in 0 ..< 10 { await Task.yield() }
                    counter.leave()
                    await gate.release()
                }
            }
            var count = 0
            for await _ in group { count += 1 }
            return count
        }

        XCTAssertEqual(completed, 30)
        XCTAssertLessThanOrEqual(counter.peak, 3)
        XCTAssertGreaterThanOrEqual(counter.peak, 1)
    }
}
