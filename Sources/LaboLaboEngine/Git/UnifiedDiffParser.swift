import Foundation

/// One file's worth of parsed `git diff` output.
public struct FileDiff: Equatable, Sendable {
    public var oldPath: String?
    public var newPath: String?
    public var isBinary: Bool
    public var isNew: Bool
    public var isDeleted: Bool
    public var isRename: Bool
    public var hunks: [DiffHunk]

    public init(
        oldPath: String? = nil,
        newPath: String? = nil,
        isBinary: Bool = false,
        isNew: Bool = false,
        isDeleted: Bool = false,
        isRename: Bool = false,
        hunks: [DiffHunk] = []
    ) {
        self.oldPath = oldPath
        self.newPath = newPath
        self.isBinary = isBinary
        self.isNew = isNew
        self.isDeleted = isDeleted
        self.isRename = isRename
        self.hunks = hunks
    }

    public var displayPath: String { newPath ?? oldPath ?? "" }
    public var additions: Int { hunks.reduce(0) { $0 + $1.lines.lazy.filter { $0.kind == .addition }.count } }
    public var deletions: Int { hunks.reduce(0) { $0 + $1.lines.lazy.filter { $0.kind == .deletion }.count } }
}

public struct DiffHunk: Equatable, Sendable {
    public var header: String
    public var oldStart: Int
    public var oldCount: Int
    public var newStart: Int
    public var newCount: Int
    public var lines: [DiffLine]

    public init(header: String, oldStart: Int, oldCount: Int, newStart: Int, newCount: Int, lines: [DiffLine]) {
        self.header = header
        self.oldStart = oldStart
        self.oldCount = oldCount
        self.newStart = newStart
        self.newCount = newCount
        self.lines = lines
    }
}

public struct DiffLine: Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        case context
        case addition
        case deletion
        case noNewline
    }

    public var kind: Kind
    public var text: String
    public var oldLineNumber: Int?
    public var newLineNumber: Int?

    public init(kind: Kind, text: String, oldLineNumber: Int? = nil, newLineNumber: Int? = nil) {
        self.kind = kind
        self.text = text
        self.oldLineNumber = oldLineNumber
        self.newLineNumber = newLineNumber
    }
}

/// Parser for unified `git diff` / `git diff --cached` output (possibly multi-file).
public enum UnifiedDiffParser {

    public static func parse(_ raw: String) -> [FileDiff] {
        var files: [FileDiff] = []
        var current: FileDiff?
        var hunk: DiffHunk?
        var oldLine = 0
        var newLine = 0

        func flushHunk() {
            if let h = hunk { current?.hunks.append(h); hunk = nil }
        }
        func flushFile() {
            flushHunk()
            if let c = current { files.append(c); current = nil }
        }

        for line in raw.split(separator: "\n", omittingEmptySubsequences: false).map(String.init) {
            if line.hasPrefix("diff --git ") {
                flushFile()
                current = FileDiff()
            } else if line.hasPrefix("--- ") {
                current?.oldPath = path(from: line, prefix: "--- ")
            } else if line.hasPrefix("+++ ") {
                current?.newPath = path(from: line, prefix: "+++ ")
            } else if line.hasPrefix("new file mode") {
                current?.isNew = true
            } else if line.hasPrefix("deleted file mode") {
                current?.isDeleted = true
            } else if line.hasPrefix("rename from ") {
                current?.isRename = true
                current?.oldPath = String(line.dropFirst("rename from ".count))
            } else if line.hasPrefix("rename to ") {
                current?.isRename = true
                current?.newPath = String(line.dropFirst("rename to ".count))
            } else if line.hasPrefix("Binary files ") || line.hasPrefix("GIT binary patch") {
                current?.isBinary = true
            } else if line.hasPrefix("@@") {
                flushHunk()
                let range = parseHunkHeader(line)
                hunk = DiffHunk(
                    header: line,
                    oldStart: range.oldStart, oldCount: range.oldCount,
                    newStart: range.newStart, newCount: range.newCount,
                    lines: []
                )
                oldLine = range.oldStart
                newLine = range.newStart
            } else if hunk != nil {
                switch line.first {
                case "+":
                    hunk?.lines.append(DiffLine(kind: .addition, text: String(line.dropFirst()), newLineNumber: newLine))
                    newLine += 1
                case "-":
                    hunk?.lines.append(DiffLine(kind: .deletion, text: String(line.dropFirst()), oldLineNumber: oldLine))
                    oldLine += 1
                case " ":
                    hunk?.lines.append(DiffLine(kind: .context, text: String(line.dropFirst()), oldLineNumber: oldLine, newLineNumber: newLine))
                    oldLine += 1
                    newLine += 1
                case "\\":
                    // "\ No newline at end of file"
                    hunk?.lines.append(DiffLine(kind: .noNewline, text: String(line.dropFirst(2))))
                default:
                    break  // blank trailing artifacts / unknown markers
                }
            }
        }
        flushFile()
        return files
    }

    // MARK: - Helpers

    private static func path(from line: String, prefix: String) -> String? {
        var p = String(line.dropFirst(prefix.count))
        if let tab = p.firstIndex(of: "\t") { p = String(p[..<tab]) }
        if p == "/dev/null" { return nil }
        if p.hasPrefix("a/") || p.hasPrefix("b/") { p = String(p.dropFirst(2)) }
        return p
    }

    private static func parseHunkHeader(_ line: String) -> (oldStart: Int, oldCount: Int, newStart: Int, newCount: Int) {
        let comps = line.split(separator: " ")
        guard comps.count >= 3 else { return (0, 1, 0, 1) }
        let old = parseRange(comps[1].dropFirst())   // strip '-'
        let new = parseRange(comps[2].dropFirst())   // strip '+'
        return (old.start, old.count, new.start, new.count)
    }

    private static func parseRange(_ s: Substring) -> (start: Int, count: Int) {
        let parts = s.split(separator: ",")
        let start = parts.isEmpty ? 0 : (Int(parts[0]) ?? 0)
        let count = parts.count > 1 ? (Int(parts[1]) ?? 1) : 1
        return (start, count)
    }
}
