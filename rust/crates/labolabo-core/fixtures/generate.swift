// Golden-fixture generator for the Rust port of LaboLaboEngine's pure parsers.
//
// This script is the "Swift oracle": it feeds every file under
// fixtures/inputs/{porcelain,diff,worktree,transcript_usage,agent_event}/
// through the real Swift parsers (Sources/LaboLaboEngine/{Git,Agent}/*.swift)
// and writes a canonical JSON representation of the result to
// fixtures/expected/<same-subdir>/<same-stem>.json.
//
// It is NOT part of the SwiftPM package graph (no executable target was added
// to Package.swift, per the porting brief) and is not built by `swift build`
// or `swift test`. It links directly against the already-compiled object
// files for the ported Swift sources (which depend on nothing outside
// Foundation — verified with `nm -g` before wiring each one in; notably
// CommitGraph.swift's `GitEngine.commitGraph` extension pulls in `GitRunner`
// and is NOT linkable this way, which is why the commit-graph pure-algorithm
// port has no golden fixtures — see ../README.md), so it can run as an
// ordinary `swiftc`-compiled one-off binary. See ../README.md for the exact
// commands to regenerate.
//
// Canonical JSON rules (must match the Rust side's `tests/golden.rs`
// serde_json canonicalization exactly, byte for byte):
//   - Compact form: no whitespace around ':' or ','.
//   - Object keys sorted lexicographically (byte order over the UTF-8 key).
//   - Optional/absent values are OMITTED as a key, never emitted as `null`.
//   - Integers rendered as plain base-10 (no leading '+', no grouping).
//   - Strings escaped with the minimal JSON escapes: " \ and control chars
//     (\n \r \t \b \f, everything else < 0x20 as \u00XX). Everything else
//     (including all non-ASCII UTF-8) passes through unescaped.
//   - A value that is entirely absent at the top level (e.g. AgentEventParser
//     dropping an event) renders as the literal `null` (this is the one place
//     `null` is used deliberately, since there is no "key" to omit).
//   - Floating-point values (e.g. estimated cost) are intentionally NOT part
//     of any golden fixture -- Swift's and Rust's default Double/f64 string
//     formatting are not guaranteed byte-identical, so cost estimation is
//     instead covered by the ported-1:1 unit tests (with a numeric
//     tolerance), not golden fixtures.

import Foundation
import LaboLaboEngine

// MARK: - Minimal canonical JSON value + encoder

indirect enum JSONValue {
    case string(String)
    case int(Int)
    case bool(Bool)
    case null
    case array([JSONValue])
    case object([(String, JSONValue)])
}

func escapeJSONString(_ s: String) -> String {
    var out = ""
    out.reserveCapacity(s.count + 2)
    out.append("\"")
    for scalar in s.unicodeScalars {
        switch scalar {
        case "\"": out.append("\\\"")
        case "\\": out.append("\\\\")
        case "\n": out.append("\\n")
        case "\r": out.append("\\r")
        case "\t": out.append("\\t")
        default:
            if scalar.value < 0x20 {
                out.append(String(format: "\\u%04x", scalar.value))
            } else {
                out.unicodeScalars.append(scalar)
            }
        }
    }
    out.append("\"")
    return out
}

func render(_ value: JSONValue) -> String {
    switch value {
    case .string(let s):
        return escapeJSONString(s)
    case .int(let i):
        return String(i)
    case .bool(let b):
        return b ? "true" : "false"
    case .null:
        return "null"
    case .array(let items):
        return "[" + items.map(render).joined(separator: ",") + "]"
    case .object(let pairs):
        let sorted = pairs.sorted { $0.0 < $1.0 }
        let body = sorted.map { "\(escapeJSONString($0.0)):\(render($0.1))" }.joined(separator: ",")
        return "{" + body + "}"
    }
}

// MARK: - Domain -> JSONValue mapping

func jsonChange(_ c: GitFileEntry.Change) -> JSONValue { .string(String(c.rawValue)) }

func jsonKind(_ k: GitFileEntry.Kind) -> JSONValue {
    switch k {
    case .ordinary: return .string("ordinary")
    case .renamedOrCopied: return .string("renamedOrCopied")
    case .unmerged: return .string("unmerged")
    case .untracked: return .string("untracked")
    case .ignored: return .string("ignored")
    }
}

func jsonEntry(_ e: GitFileEntry) -> JSONValue {
    var fields: [(String, JSONValue)] = [
        ("index", jsonChange(e.index)),
        ("kind", jsonKind(e.kind)),
        ("path", .string(e.path)),
        ("worktree", jsonChange(e.worktree)),
    ]
    if let originalPath = e.originalPath { fields.append(("originalPath", .string(originalPath))) }
    if let score = e.score { fields.append(("score", .int(score))) }
    return .object(fields)
}

func jsonStatus(_ status: GitStatus) -> JSONValue {
    var fields: [(String, JSONValue)] = [
        ("ahead", .int(status.ahead)),
        ("behind", .int(status.behind)),
        ("conflicted", .array(status.conflicted.map { .string($0.path) })),
        ("entries", .array(status.entries.map(jsonEntry))),
        ("isDetached", .bool(status.isDetached)),
        ("isDirty", .bool(status.isDirty)),
        ("staged", .array(status.staged.map { .string($0.path) })),
        ("unstaged", .array(status.unstaged.map { .string($0.path) })),
        ("untracked", .array(status.untracked.map { .string($0.path) })),
    ]
    if let headSha = status.headSha { fields.append(("headSha", .string(headSha))) }
    if let branch = status.branch { fields.append(("branch", .string(branch))) }
    if let upstream = status.upstream { fields.append(("upstream", .string(upstream))) }
    return .object(fields)
}

func jsonLineKind(_ k: DiffLine.Kind) -> JSONValue {
    switch k {
    case .context: return .string("context")
    case .addition: return .string("addition")
    case .deletion: return .string("deletion")
    case .noNewline: return .string("noNewline")
    }
}

func jsonLine(_ l: DiffLine) -> JSONValue {
    var fields: [(String, JSONValue)] = [
        ("kind", jsonLineKind(l.kind)),
        ("text", .string(l.text)),
    ]
    if let old = l.oldLineNumber { fields.append(("oldLineNumber", .int(old))) }
    if let new = l.newLineNumber { fields.append(("newLineNumber", .int(new))) }
    return .object(fields)
}

func jsonHunk(_ h: DiffHunk) -> JSONValue {
    .object([
        ("header", .string(h.header)),
        ("lines", .array(h.lines.map(jsonLine))),
        ("newCount", .int(h.newCount)),
        ("newStart", .int(h.newStart)),
        ("oldCount", .int(h.oldCount)),
        ("oldStart", .int(h.oldStart)),
    ])
}

// MARK: - Wave 2: Worktree

func jsonWorktree(_ w: Worktree) -> JSONValue {
    var fields: [(String, JSONValue)] = [
        ("path", .string(w.path)),
        ("isDetached", .bool(w.isDetached)),
        ("isLocked", .bool(w.isLocked)),
        ("isBare", .bool(w.isBare)),
    ]
    if let head = w.head { fields.append(("head", .string(head))) }
    if let branch = w.branch { fields.append(("branch", .string(branch))) }
    if let shortBranch = w.shortBranch { fields.append(("shortBranch", .string(shortBranch))) }
    return .object(fields)
}

// MARK: - Wave 2: TranscriptUsage
//
// estimatedCostUSD is intentionally excluded -- see the module doc comment
// at the top of this file for why (float-formatting is not guaranteed
// byte-identical between Swift and Rust; cost estimation is covered by
// ported unit tests with a numeric tolerance instead).

func jsonAgentUsage(_ u: AgentUsage) -> JSONValue {
    var fields: [(String, JSONValue)] = [
        ("inputTokens", .int(u.inputTokens)),
        ("outputTokens", .int(u.outputTokens)),
        ("cacheCreationTokens", .int(u.cacheCreationTokens)),
        ("cacheReadTokens", .int(u.cacheReadTokens)),
        ("turns", .int(u.turns)),
        ("totalTokens", .int(u.totalTokens)),
        ("isEmpty", .bool(u.isEmpty)),
    ]
    if let model = u.model { fields.append(("model", .string(model))) }
    return .object(fields)
}

// MARK: - Wave 2: AgentEventParser
//
// `nil` (event dropped) renders as the JSON literal `null`; see the module
// doc comment for why this is the one place `null` is used deliberately.

func jsonAgentStatusEvent(_ event: AgentStatusEvent?) -> JSONValue {
    guard let event = event else { return .null }
    var fields: [(String, JSONValue)] = [
        ("hookEvent", .string(event.hookEvent)),
        ("status", .string(event.status.rawValue)),
    ]
    if let sessionID = event.sessionID { fields.append(("sessionID", .string(sessionID))) }
    if let transcriptPath = event.transcriptPath { fields.append(("transcriptPath", .string(transcriptPath))) }
    if let cwd = event.cwd { fields.append(("cwd", .string(cwd))) }
    if let paneID = event.paneID { fields.append(("paneID", .string(paneID))) }
    return .object(fields)
}

func jsonFileDiff(_ f: FileDiff) -> JSONValue {
    var fields: [(String, JSONValue)] = [
        ("additions", .int(f.additions)),
        ("deletions", .int(f.deletions)),
        ("displayPath", .string(f.displayPath)),
        ("hunks", .array(f.hunks.map(jsonHunk))),
        ("isBinary", .bool(f.isBinary)),
        ("isDeleted", .bool(f.isDeleted)),
        ("isNew", .bool(f.isNew)),
        ("isRename", .bool(f.isRename)),
    ]
    if let newPath = f.newPath { fields.append(("newPath", .string(newPath))) }
    if let oldPath = f.oldPath { fields.append(("oldPath", .string(oldPath))) }
    return .object(fields)
}

// MARK: - Driver

let fm = FileManager.default
let fixturesRoot = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : ".")
let inputsRoot = fixturesRoot.appendingPathComponent("inputs")
let expectedRoot = fixturesRoot.appendingPathComponent("expected")

func readRawString(_ url: URL) throws -> String {
    let data = try Data(contentsOf: url)
    return String(decoding: data, as: UTF8.self)
}

func processDirectory(_ subpath: String, parse: (String) -> JSONValue) throws -> Int {
    let inputDir = inputsRoot.appendingPathComponent(subpath)
    let outputDir = expectedRoot.appendingPathComponent(subpath)
    try fm.createDirectory(at: outputDir, withIntermediateDirectories: true)
    let entries = try fm.contentsOfDirectory(at: inputDir, includingPropertiesForKeys: nil)
        .sorted { $0.lastPathComponent < $1.lastPathComponent }
    var count = 0
    for entry in entries {
        guard entry.pathExtension != "json" else { continue }
        let raw = try readRawString(entry)
        let json = parse(raw)
        let rendered = render(json)
        let stem = entry.deletingPathExtension().lastPathComponent
        let outURL = outputDir.appendingPathComponent(stem + ".json")
        try rendered.write(to: outURL, atomically: true, encoding: .utf8)
        count += 1
    }
    return count
}

do {
    let porcelainCount = try processDirectory("porcelain") { raw in
        jsonStatus(PorcelainStatusParser.parse(raw))
    }
    let diffCount = try processDirectory("diff") { raw in
        .array(UnifiedDiffParser.parse(raw).map(jsonFileDiff))
    }
    let worktreeCount = try processDirectory("worktree") { raw in
        .array(WorktreeListParser.parse(raw).map(jsonWorktree))
    }
    let transcriptUsageCount = try processDirectory("transcript_usage") { raw in
        jsonAgentUsage(TranscriptUsage.parse(jsonl: raw))
    }
    let agentEventCount = try processDirectory("agent_event") { raw in
        jsonAgentStatusEvent(AgentEventParser.parse(Data(raw.utf8)))
    }
    print("""
    generated \(porcelainCount) porcelain, \(diffCount) diff, \(worktreeCount) worktree, \
    \(transcriptUsageCount) transcript_usage, \(agentEventCount) agent_event expected fixtures
    """)
} catch {
    FileHandle.standardError.write("golden fixture generation failed: \(error)\n".data(using: .utf8)!)
    exit(1)
}
