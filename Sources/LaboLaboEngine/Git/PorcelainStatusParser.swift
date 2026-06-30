import Foundation

/// Parser for `git status --porcelain=v2 --branch -z`.
///
/// Records are NUL-separated. Rename/copy (type `2`) entries store the original
/// path in the *following* NUL token, so the tokenizer must consume two tokens
/// for those. See: https://git-scm.com/docs/git-status#_porcelain_format_version_2
public enum PorcelainStatusParser {

    public static func parse(_ raw: String) -> GitStatus {
        var status = GitStatus()
        let tokens = raw
            .split(separator: "\u{0}", omittingEmptySubsequences: true)
            .map(String.init)

        var i = 0
        while i < tokens.count {
            let token = tokens[i]
            switch token.first {
            case "#":
                parseHeader(token, into: &status)
                i += 1
            case "1":
                if let entry = parseOrdinary(token) { status.entries.append(entry) }
                i += 1
            case "2":
                let original = (i + 1 < tokens.count) ? tokens[i + 1] : nil
                if let entry = parseRenameCopy(token, originalPath: original) {
                    status.entries.append(entry)
                }
                i += 2
            case "u":
                if let entry = parseUnmerged(token) { status.entries.append(entry) }
                i += 1
            case "?":
                status.entries.append(GitFileEntry(kind: .untracked, path: String(token.dropFirst(2))))
                i += 1
            case "!":
                status.entries.append(GitFileEntry(kind: .ignored, path: String(token.dropFirst(2))))
                i += 1
            default:
                i += 1
            }
        }
        return status
    }

    // MARK: - Header

    private static func parseHeader(_ token: String, into status: inout GitStatus) {
        let parts = token.split(separator: " ")
        guard parts.count >= 3 else { return }
        switch parts[1] {
        case "branch.oid":
            status.headSha = parts[2] == "(initial)" ? nil : String(parts[2])
        case "branch.head":
            status.branch = String(parts[2])
        case "branch.upstream":
            status.upstream = String(parts[2])
        case "branch.ab" where parts.count >= 4:
            status.ahead = Int(parts[2].dropFirst()) ?? 0   // "+N"
            status.behind = Int(parts[3].dropFirst()) ?? 0  // "-M"
        default:
            break
        }
    }

    // MARK: - Entries

    /// `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`
    private static func parseOrdinary(_ token: String) -> GitFileEntry? {
        let f = token.split(separator: " ", maxSplits: 8, omittingEmptySubsequences: false)
        guard f.count >= 9, let xy = xyPair(f[1]) else { return nil }
        return GitFileEntry(
            kind: .ordinary,
            index: xy.0,
            worktree: xy.1,
            path: String(f[8])
        )
    }

    /// `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <Xscore> <path>` (+ original path in next token)
    private static func parseRenameCopy(_ token: String, originalPath: String?) -> GitFileEntry? {
        let f = token.split(separator: " ", maxSplits: 9, omittingEmptySubsequences: false)
        guard f.count >= 10, let xy = xyPair(f[1]) else { return nil }
        let xscore = f[8]                       // e.g. "R100" / "C75"
        let score = xscore.count >= 2 ? Int(xscore.dropFirst()) : nil
        return GitFileEntry(
            kind: .renamedOrCopied,
            index: xy.0,
            worktree: xy.1,
            path: String(f[9]),
            originalPath: originalPath,
            score: score
        )
    }

    /// `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`
    private static func parseUnmerged(_ token: String) -> GitFileEntry? {
        let f = token.split(separator: " ", maxSplits: 10, omittingEmptySubsequences: false)
        guard f.count >= 11, let xy = xyPair(f[1]) else { return nil }
        return GitFileEntry(
            kind: .unmerged,
            index: xy.0,
            worktree: xy.1,
            path: String(f[10])
        )
    }

    private static func xyPair(_ field: Substring) -> (GitFileEntry.Change, GitFileEntry.Change)? {
        let chars = Array(field)
        guard chars.count == 2 else { return nil }
        return (GitFileEntry.Change(porcelain: chars[0]), GitFileEntry.Change(porcelain: chars[1]))
    }
}
