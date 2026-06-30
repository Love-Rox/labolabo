import XCTest
@testable import LaboLaboEngine

final class UnifiedDiffParserTests: XCTestCase {

    func testModifiedFileHunkAndLineNumbers() {
        let raw = """
        diff --git a/src/foo.swift b/src/foo.swift
        index 1111111..2222222 100644
        --- a/src/foo.swift
        +++ b/src/foo.swift
        @@ -1,3 +1,4 @@
         line1
        -line2
        +line2 changed
        +line3 added
         line4
        """
        let files = UnifiedDiffParser.parse(raw)
        XCTAssertEqual(files.count, 1)

        let file = files[0]
        XCTAssertEqual(file.oldPath, "src/foo.swift")
        XCTAssertEqual(file.newPath, "src/foo.swift")
        XCTAssertEqual(file.displayPath, "src/foo.swift")
        XCTAssertFalse(file.isBinary)
        XCTAssertEqual(file.additions, 2)
        XCTAssertEqual(file.deletions, 1)

        XCTAssertEqual(file.hunks.count, 1)
        let hunk = file.hunks[0]
        XCTAssertEqual(hunk.oldStart, 1)
        XCTAssertEqual(hunk.oldCount, 3)
        XCTAssertEqual(hunk.newStart, 1)
        XCTAssertEqual(hunk.newCount, 4)
        XCTAssertEqual(hunk.lines.map(\.kind), [.context, .deletion, .addition, .addition, .context])

        // Line numbering: context keeps both, addition only new, deletion only old.
        XCTAssertEqual(hunk.lines[0].oldLineNumber, 1)
        XCTAssertEqual(hunk.lines[0].newLineNumber, 1)
        XCTAssertEqual(hunk.lines[1].oldLineNumber, 2)
        XCTAssertNil(hunk.lines[1].newLineNumber)
        XCTAssertEqual(hunk.lines[2].newLineNumber, 2)
        XCTAssertNil(hunk.lines[2].oldLineNumber)
        XCTAssertEqual(hunk.lines[4].oldLineNumber, 3)
        XCTAssertEqual(hunk.lines[4].newLineNumber, 4)
    }

    func testNewFile() {
        let raw = """
        diff --git a/new.txt b/new.txt
        new file mode 100644
        index 0000000..abc1234
        --- /dev/null
        +++ b/new.txt
        @@ -0,0 +1,2 @@
        +hello
        +world
        """
        let files = UnifiedDiffParser.parse(raw)
        XCTAssertEqual(files.count, 1)
        let file = files[0]
        XCTAssertTrue(file.isNew)
        XCTAssertNil(file.oldPath, "/dev/null maps to nil")
        XCTAssertEqual(file.newPath, "new.txt")
        XCTAssertEqual(file.additions, 2)
        XCTAssertEqual(file.deletions, 0)
    }

    func testBinaryFile() {
        let raw = """
        diff --git a/img.png b/img.png
        index aaaaaaa..bbbbbbb 100644
        Binary files a/img.png and b/img.png differ
        """
        let files = UnifiedDiffParser.parse(raw)
        XCTAssertEqual(files.count, 1)
        XCTAssertTrue(files[0].isBinary)
        XCTAssertTrue(files[0].hunks.isEmpty)
    }

    func testMultipleFiles() {
        let raw = """
        diff --git a/a.txt b/a.txt
        --- a/a.txt
        +++ b/a.txt
        @@ -1 +1 @@
        -old
        +new
        diff --git a/b.txt b/b.txt
        --- a/b.txt
        +++ b/b.txt
        @@ -1,2 +1,2 @@
         keep
        -drop
        +add
        """
        let files = UnifiedDiffParser.parse(raw)
        XCTAssertEqual(files.map(\.displayPath), ["a.txt", "b.txt"])
        XCTAssertEqual(files[0].hunks.count, 1)
        XCTAssertEqual(files[1].hunks.count, 1)
    }
}
