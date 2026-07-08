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

    /// 現在ブランチから PR を作成し、PR の URL を返す（`gh pr create`）。
    /// 事前に現在ブランチが push 済みであること（`GitEngine.push`）。
    public func createPullRequest(
        worktree: URL, base: String, title: String, body: String, draft: Bool
    ) async throws -> String {
        guard let gh = Self.locateGH() else {
            throw NSError(domain: "GitHubEngine", code: 127, userInfo: [
                NSLocalizedDescriptionKey: "gh CLI が見つかりません（brew install gh）",
            ])
        }
        var args = ["pr", "create", "--base", base, "--title", title, "--body", body]
        if draft { args.append("--draft") }
        let output = try await Self.run(executable: gh, arguments: args, in: worktree)
        return Self.parsePRURL(from: output)
    }

    /// `gh pr create` の stdout から PR の URL を取り出す。gh は URL を出すが、前後に
    /// 助言行が混ざることがあるので http(s):// 始まりの行を優先し、無ければ全体を trim。
    static func parsePRURL(from output: String) -> String {
        let urlLine = output
            .split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .last(where: { $0.hasPrefix("https://") || $0.hasPrefix("http://") })
        return urlLine ?? output.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    // MARK: - gh の場所

    /// gh の絶対パス。`ToolLocator` に集約し、doctor の判定と実際の起動可否を一致させる。
    static func locateGH() -> URL? {
        ToolLocator.locate("gh")
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

    // MARK: - プロセス実行（stdout を返す。スレッド非占有の ProcessRunner に委譲）

    static func run(executable: URL, arguments: [String], in directory: URL) async throws -> String {
        let output = try await ProcessRunner.run(
            executable: executable, arguments: arguments, in: directory
        )
        guard output.status == 0 else {
            // stderr を載せて UI に理由を出せるようにする（未認証・重複 PR など）。
            let stderr = output.stderr.trimmingCharacters(in: .whitespacesAndNewlines)
            throw NSError(
                domain: "GitHubEngine", code: Int(output.status),
                userInfo: stderr.isEmpty ? nil : [NSLocalizedDescriptionKey: stderr]
            )
        }
        return output.stdout
    }
}
