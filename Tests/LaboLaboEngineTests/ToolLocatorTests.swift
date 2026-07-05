import XCTest
@testable import LaboLaboEngine

final class ToolLocatorTests: XCTestCase {

    // ベースシステムの `sh` は必ず存在する（macOS では /bin/sh、PATH 経由で解決）。
    // 解決手段（固定候補 / PATH / ログインシェル）に依らず、返る URL の実体が
    // 実行可能ファイルであることを検証する。
    func testLocateBaseBinaryReturnsExecutableURL() {
        let url = ToolLocator.locate("sh")
        XCTAssertNotNil(url, "ベース binary `sh` は解決できるはず")
        guard let url else { return }

        XCTAssertTrue(url.isFileURL, "file URL であること")
        XCTAssertTrue(url.path.hasPrefix("/"), "絶対パスであること: \(url.path)")
        XCTAssertTrue(
            FileManager.default.isExecutableFile(atPath: url.path),
            "返された URL は実行可能ファイルを指すはず: \(url.path)"
        )
    }

    // 別のベース binary（`ls`）でも同様に解決でき、実体が存在・実行可能であること。
    func testLocateAnotherBaseBinaryPointsToExistingExecutable() {
        let url = ToolLocator.locate("ls")
        XCTAssertNotNil(url, "ベース binary `ls` は解決できるはず")
        guard let url else { return }

        var isDir: ObjCBool = true
        XCTAssertTrue(
            FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir),
            "解決先は実在するファイル: \(url.path)"
        )
        XCTAssertFalse(isDir.boolValue, "ディレクトリではなくファイルを指すはず: \(url.path)")
        XCTAssertTrue(FileManager.default.isExecutableFile(atPath: url.path))
    }

    // 存在し得ないツール名は nil を返す（固定候補・PATH・ログインシェルすべて外れる）。
    //
    // 有効な名前（英数）は許可リストを通るのでログインシェル（`$SHELL -l -c 'command -v …'`）
    // まで到達する。CI ランナーによってはログイン profile がバックグラウンド常駐を起こし、
    // その孫プロセスがパイプを握って ProcessRunner がハング → `swift test` が無限に止まる。
    // 単体テストで実ログインシェルを起こすのは非ハーメティックなので、**CI では skip** する
    // （ローカルでは実行して回帰を守る）。実行時のハング自体は ProcessRunner 側で有限化済み。
    func testLocateAbsentToolReturnsNil() throws {
        try XCTSkipIf(
            ProcessInfo.processInfo.environment["CI"] != nil,
            "CI ではログインシェル起動を避ける（非ハーメティック・ハング要因）"
        )
        let name = "labolabo-no-such-tool-\(UUID().uuidString)"
        XCTAssertNil(ToolLocator.locate(name), "存在しないツールは nil を返すはず")
    }

    // ログインシェルの許可リスト（英数と - _ のみ）を通らない名前（空白入り）でも、
    // クラッシュせずに固定候補フォールバックで解決するか、なければ nil を返す。
    func testLocateNameFailingLoginShellAllowListDoesNotCrash() {
        // 空白を含むため login shell 解決はスキップされる。固定候補・PATH にも該当なし。
        let url = ToolLocator.locate("bad name")
        // 通常は該当ファイルが無いので nil。仮に返った場合でも実行可能ファイルを指すこと。
        if let url {
            XCTAssertTrue(
                FileManager.default.isExecutableFile(atPath: url.path),
                "非 nil の場合は実行可能ファイルを指すこと: \(url.path)"
            )
        } else {
            XCTAssertNil(url)
        }
    }

    // 許可リストを通らない他の記号（スラッシュ・ドット）を含む名前でもクラッシュしない。
    func testLocateNameWithPathSeparatorsDoesNotCrash() {
        XCTAssertNil(
            ToolLocator.locate("../etc/passwd"),
            "パス区切りを含む名前はどの候補にも一致せず nil を返すはず"
        )
        // 単独のドットやスラッシュでも例外なく nil。
        XCTAssertNil(ToolLocator.locate("no.such.tool/here"))
    }
}
