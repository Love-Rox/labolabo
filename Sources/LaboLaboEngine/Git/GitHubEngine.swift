import Foundation

/// 1 つの worktree のブランチに対応する Pull Request の要約。
public struct PullRequestInfo: Sendable, Equatable {
    public enum State: String, Sendable {
        case open, draft, merged, closed
    }

    public enum Checks: String, Sendable {
        case passing, failing, pending, none
    }

    public let number: Int
    public let title: String
    public let state: State
    public let checks: Checks
    public let url: String
    /// PR 本文から推定した関連 Issue 番号（無ければ nil）。
    public let issue: Int?

    public init(number: Int, title: String, state: State, checks: Checks, url: String, issue: Int?) {
        self.number = number
        self.title = title
        self.state = state
        self.checks = checks
        self.url = url
        self.issue = issue
    }
}

/// `gh` CLI 経由で GitHub 情報を取得する。`gh` が見つからない/未認証/PR 無しの場合は
/// nil を返す（UI 側は git のブランチ情報にフォールバックする）。
public actor GitHubEngine {
    public init() {}

    public func pullRequest(worktree: URL) async throws -> PullRequestInfo? {
        guard let gh = Self.locateGH() else { return nil }
        let fields = "number,title,state,isDraft,url,statusCheckRollup,body"
        let output: String
        do {
            output = try await Self.run(
                executable: gh,
                arguments: ["pr", "view", "--json", fields],
                in: worktree
            )
        } catch {
            return nil // PR 無し / 未認証 / ネット無し など
        }
        return Self.parse(output)
    }

    // MARK: - gh の場所

    static func locateGH() -> URL? {
        let fm = FileManager.default
        let candidates = [
            "/opt/homebrew/bin/gh",
            "/usr/local/bin/gh",
            "/usr/bin/gh",
            "/run/current-system/sw/bin/gh",
        ]
        for path in candidates where fm.isExecutableFile(atPath: path) {
            return URL(fileURLWithPath: path)
        }
        if let pathEnv = ProcessInfo.processInfo.environment["PATH"] {
            for dir in pathEnv.split(separator: ":") {
                let path = String(dir) + "/gh"
                if fm.isExecutableFile(atPath: path) { return URL(fileURLWithPath: path) }
            }
        }
        return nil
    }

    // MARK: - パース

    static func parse(_ json: String) -> PullRequestInfo? {
        guard let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let number = object["number"] as? Int else { return nil }

        let title = object["title"] as? String ?? ""
        let url = object["url"] as? String ?? ""
        let isDraft = object["isDraft"] as? Bool ?? false

        let state: PullRequestInfo.State
        switch (object["state"] as? String ?? "OPEN").uppercased() {
        case "MERGED": state = .merged
        case "CLOSED": state = .closed
        default: state = isDraft ? .draft : .open
        }

        let checks = parseChecks(object["statusCheckRollup"])
        let issue = parseIssue(fromBody: object["body"] as? String ?? "")
        return PullRequestInfo(number: number, title: title, state: state, checks: checks, url: url, issue: issue)
    }

    static func parseChecks(_ raw: Any?) -> PullRequestInfo.Checks {
        guard let items = raw as? [[String: Any]], !items.isEmpty else { return .none }
        var anyFail = false, anyPending = false, anySuccess = false
        for item in items {
            let conclusion = (item["conclusion"] as? String ?? "").uppercased()
            let status = (item["status"] as? String ?? "").uppercased()
            let stateField = (item["state"] as? String ?? "").uppercased()

            if ["FAILURE", "ERROR", "TIMED_OUT", "CANCELLED", "ACTION_REQUIRED", "STARTUP_FAILURE"].contains(conclusion)
                || stateField == "FAILURE" || stateField == "ERROR" {
                anyFail = true
            } else if conclusion == "SUCCESS" || stateField == "SUCCESS" {
                anySuccess = true
            } else {
                // IN_PROGRESS / QUEUED / PENDING / 空 など
                anyPending = true
            }
        }
        if anyFail { return .failing }
        if anyPending { return .pending }
        if anySuccess { return .passing }
        return .none
    }

    static func parseIssue(fromBody body: String) -> Int? {
        let pattern = #"(?i)(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)"#
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return nil }
        let range = NSRange(body.startIndex..., in: body)
        guard let match = regex.firstMatch(in: body, range: range),
              let group = Range(match.range(at: 1), in: body) else { return nil }
        return Int(body[group])
    }

    // MARK: - プロセス実行（stdout を返す。pipe を並行 drain してデッドロック回避）

    static func run(executable: URL, arguments: [String], in directory: URL) async throws -> String {
        try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = executable
                process.arguments = arguments
                process.currentDirectoryURL = directory

                let outPipe = Pipe()
                let errPipe = Pipe()
                process.standardOutput = outPipe
                process.standardError = errPipe

                do {
                    try process.run()
                } catch {
                    continuation.resume(throwing: error)
                    return
                }

                var outData = Data()
                let group = DispatchGroup()
                group.enter()
                DispatchQueue.global().async {
                    outData = outPipe.fileHandleForReading.readDataToEndOfFile()
                    group.leave()
                }
                group.enter()
                DispatchQueue.global().async {
                    _ = errPipe.fileHandleForReading.readDataToEndOfFile()
                    group.leave()
                }

                process.waitUntilExit()
                group.wait()

                if process.terminationStatus == 0 {
                    continuation.resume(returning: String(decoding: outData, as: UTF8.self))
                } else {
                    continuation.resume(throwing: NSError(
                        domain: "GitHubEngine", code: Int(process.terminationStatus)
                    ))
                }
            }
        }
    }
}
