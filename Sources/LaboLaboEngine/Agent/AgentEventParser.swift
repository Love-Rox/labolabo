import Foundation

/// hook フォワーダから届いた 1 メッセージ（JSON バイト列）を `AgentStatusEvent` へ解釈する。
/// トランスポート（AF_UNIX / 将来の Named Pipe 等）に依存しない純関数として分離してある —
/// クロスプラットフォーム化ではトランスポートだけを OS 別に差し替え、この解釈層と
/// ワイヤ仕様（docs/hooks-protocol.md)を全プラットフォームで共有する。
public enum AgentEventParser {
    /// 不正な JSON・未知の hook イベントは nil（呼び出し側は黙って捨てる）。
    public static func parse(_ data: Data) -> AgentStatusEvent? {
        guard !data.isEmpty,
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        let hookEvent = object["hook_event_name"] as? String ?? ""
        guard let status = AgentStatus.from(hookEvent: hookEvent) else { return nil }
        return AgentStatusEvent(
            hookEvent: hookEvent,
            status: status,
            sessionID: object["session_id"] as? String,
            transcriptPath: object["transcript_path"] as? String,
            cwd: object["cwd"] as? String,
            paneID: object["labolabo_pane_id"] as? String
        )
    }
}
