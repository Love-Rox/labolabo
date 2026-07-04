import XCTest
@testable import LaboLaboEngine

final class ReleaseVersionTests: XCTestCase {

    func testNormalizeStripsVPrefixAndWhitespace() {
        XCTAssertEqual(ReleaseVersion.normalize("v0.3.2"), "0.3.2")
        XCTAssertEqual(ReleaseVersion.normalize("V1.0"), "1.0")
        XCTAssertEqual(ReleaseVersion.normalize("  v2.1.0  "), "2.1.0")
        XCTAssertEqual(ReleaseVersion.normalize("0.3.2"), "0.3.2")
    }

    func testIsNewer() {
        XCTAssertTrue(ReleaseVersion.isNewer("v0.3.3", than: "0.3.2"))
        XCTAssertTrue(ReleaseVersion.isNewer("0.4.0", than: "0.3.9"))
        XCTAssertTrue(ReleaseVersion.isNewer("1.0.0", than: "0.9.9"))
        XCTAssertTrue(ReleaseVersion.isNewer("v0.3.10", than: "v0.3.9")) // 数値比較（辞書順でない）
    }

    func testNotNewerWhenEqualOrOlder() {
        XCTAssertFalse(ReleaseVersion.isNewer("0.3.2", than: "0.3.2"))
        XCTAssertFalse(ReleaseVersion.isNewer("v0.3.2", than: "0.3.2"))
        XCTAssertFalse(ReleaseVersion.isNewer("0.3.1", than: "0.3.2"))
        XCTAssertFalse(ReleaseVersion.isNewer("0.9.9", than: "1.0.0"))
    }

    func testDifferentSegmentCountsAreEqual() {
        XCTAssertEqual(ReleaseVersion.compare("0.3", "0.3.0"), 0)
        XCTAssertEqual(ReleaseVersion.compare("1", "1.0.0"), 0)
        XCTAssertTrue(ReleaseVersion.isNewer("0.3.1", than: "0.3"))
    }

    func testNonNumericSuffixIgnored() {
        XCTAssertEqual(ReleaseVersion.compare("0.3.2-beta", "0.3.2"), 0)
        XCTAssertTrue(ReleaseVersion.isNewer("0.3.3-rc1", than: "0.3.2"))
    }
}
