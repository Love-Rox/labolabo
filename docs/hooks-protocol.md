# LaboLabo hooks ワイヤプロトコル仕様

- **Status**: v1（2026-07-14。実装済みの挙動を仕様として固定したもの）
- **目的**: Claude Code の hooks を使ったエージェント状態検出・セッション対応付けの配線を、**実装言語・OS に依存しない形**で再実装可能にする。Rust 版（クロスプラットフォーム化）はこの文書を正として実装する。
- **実装の現在地（Swift 版）**: 解釈層 = `AgentEventParser`、トランスポート契約 = `AgentEventTransport`、AF_UNIX 実装 = `UnixSocketEventTransport`（いずれも `Sources/LaboLaboEngine/Agent/`）。フォワーダ = `app/Sources/HookForwarder.swift`。hooks 注入 = `app/Sources/AgentSessionModel.swift`。

## 1. 全体像

```
Claude Code（エージェント CLI、LaboLabo の端末ペイン内で動く）
  │  hook イベント発火（settings.local.json に注入された command hook）
  ▼
labolabo --hook <socketPath>          … フォワーダ（LaboLabo 自身の別モード起動）
  │  stdin の JSON を読み、LABOLABO_PANE 環境変数があれば labolabo_pane_id を付与
  ▼
<socketPath>（AF_UNIX, SOCK_STREAM）  … 1 接続 = 1 イベント
  │
  ▼
LaboLabo 本体（セッションごとのソケットサーバ）
  →  JSON 解釈 → AgentStatus 更新 / セッション・タブ対応付けの記録
```

## 2. hooks の注入（`settings.local.json`）

- 対象ファイル: worktree の `.claude/settings.local.json`（**worktree 単位**。gitignore 前提のローカル設定）。
- 注入タイミング: セッションのエージェント監視開始時（アプリ起動時の全セッション復元を含む）。終了時に原本へ復元する。
  - 既存ファイルがあれば内容をスナップショット（`settings.local.json.labolabo-bak`）してからマージ。終了時にバックアップから復元。ファイルが無かった場合は自分が作った印を持ち、終了時に削除。
  - 前回クラッシュ等でバックアップが残っていた場合は、注入前にまず原本へ復元してから改めてスナップショットする（二重注入防止）。
- 注入内容: 以下の **7 イベント**それぞれに同一の command hook を append する（既存 hooks は保持）:
  `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Notification`, `Stop`, `SessionEnd`
- command の形（シェルクォート済み）:
  ```
  '<LaboLabo 実行バイナリの絶対パス>' --hook '<socketPath>'
  ```
  timeout は 5 秒を指定する（Claude を待たせない）。

## 3. フォワーダ契約（`labolabo --hook <socketPath>`）

1. stdin を **EOF まで**全読みする（Claude Code が hook の stdin にイベント JSON を渡す）。
2. **ペイン ID の付与**: 環境変数 `LABOLABO_PANE` が非空で、かつ stdin が JSON オブジェクトとして解釈できる場合、トップレベルに `"labolabo_pane_id": "<値>"` を追加して再シリアライズする。変数が無い・JSON でない場合は**原文のまま**転送する。
   - フォワーダは hook として Claude の子孫プロセスで走るため、ペインのシェルに注入された環境変数を継承している（§7）。
3. `<socketPath>` へ connect → 全バイトを write → close → **即 exit(0)**。接続失敗・パス過長などあらゆる失敗も exit(0)（hook の失敗で Claude を止めない）。

## 4. トランスポート（受信側）

- **チャネル**: セッションごとに 1 本の AF_UNIX ソケット（SOCK_STREAM）。
- **socketPath**: `/tmp/labolabo/<セッション UUID の先頭 10 文字（ハイフン除去・小文字）>.sock`
  - ディレクトリ `/tmp/labolabo` は 0700 で作成。ソケットは bind 後に **0600** へ chmod（同一ユーザーのみ）。
  - 同一パスはアプリ再起動を跨いで再利用される（起動時に残骸を unlink してから bind）。
- **フレーミング**: **1 接続 = 1 イベント**。受信側は accept 後 EOF まで読み、その全体を 1 メッセージとして扱う。長さプレフィクスや区切り文字は使わない。
- **注意（既知のレース）**: アプリ再起動直後、旧プロセス由来の遅延イベント（死んだ claude の SessionEnd 等）が再 bind 済みの同一パスへ届くことがある。消費側はこれを前提に防御する（§6）。
- **Windows 代替（未実装・Spike 3 で選定）**: AF_UNIX（Windows 10 1803+、SOCK_STREAM のみ）/ Named Pipe / loopback TCP。フレーミング「1 接続 = 1 イベント」の意味論を保てばトランスポートは差し替え可能。

## 5. イベント JSON

受信側が解釈するトップレベルフィールド（**すべて文字列**。他のフィールドは無視 = 前方互換）:

| フィールド | 必須 | 意味 |
|---|---|---|
| `hook_event_name` | ✔ | Claude Code の hook イベント名。未知の値は**イベントごと破棄** |
| `session_id` | – | Claude セッション ID（`claude --resume` に使う） |
| `transcript_path` | – | transcript(JSONL) の絶対パス |
| `cwd` | – | イベント発生時の作業ディレクトリ |
| `labolabo_pane_id` | – | フォワーダが付与した端末ペイン UUID（§3）。外部ターミナル起動分には付かない |

### イベント → 状態のマッピング（正: `AgentStatus.from(hookEvent:)`）

| hook_event_name | AgentStatus |
|---|---|
| `SessionStart` | `starting` |
| `UserPromptSubmit` / `PreToolUse` / `PostToolUse` | `running` |
| `Notification` | `waitingForInput`（入力・許可待ち） |
| `Stop` / `SubagentStop` | `idle`（応答完了・待機） |
| `SessionEnd` | `ended` |
| それ以外 | （破棄） |

破棄規則: 空ペイロード / JSON として不正 / `hook_event_name` 欠落・未知 → 黙って捨てる（ログも状態遷移もしない）。

## 6. 消費側の意味論（受信後にアプリが行うこと）

- **状態**: 最後に受けたイベントの `AgentStatus` をセッションの現在状態とする（**last-writer-wins**。同一 worktree で複数の claude が動くと状態は混合する — 既知の制限）。
- **セッション ID の記録**: `session_id` 付きイベントを受けるたび、(a) セッション（worktree）単位の「最後の ID」を永続化（次回 `--resume` のフォールバック用）、(b) `labolabo_pane_id` があれば **（ペイン, session_id, transcript_path）の対応**をレイアウトと一緒に永続化（タブ別 resume 用）。同値なら再保存しない。
- **タブ別 auto-resume のガード**: 復元時、ペインに記録された `transcript_path` が実在しない場合は resume を打たない（会話を保存せず終了した空セッションの ID への無駄打ち防止）。パス未記録（旧データ）は従来どおり試す。
- **遅延 SessionEnd への防御**: 起動直後の auto-resume 判定は、状態が `none` **または `ended`** のとき実行してよい（§4 のレースで `ended` になっていても resume を妨げない）。
- **使用量集計**: `idle`/`ended` 到達時に `transcript_path` の JSONL を読んで使用量を推定（best-effort）。

## 7. `LABOLABO_PANE` 注入契約

- **契約**: LaboLabo が生成した端末ペインで起動されるすべてのプロセス（手打ちの `claude` を含む）は、環境変数 `LABOLABO_PANE=<ペイン UUID>` を継承していること。
- **現実装（Swift 版）**: ペイン専用の Ghostty generated config で
  `command = /usr/bin/env LABOLABO_PANE=<uuid> <ログインシェル> -l` に差し替えて実現（POSIX 前提のワークアラウンド。ユーザー config の `command` は意図的に無視）。
- **新版（Rust）での実現**: PTY spawn 時の環境変数ブロックへ直接注入する（ワークアラウンド不要・全 OS 同一）。
- **予約**: 作業（タスク）モデル導入時に `LABOLABO_TASK=<タスク UUID>` を同様に注入する（plans/012。同一 cwd に複数タスクが並ぶ場合のルーティング用。受信側フィールド名は `labolabo_task_id` を予約）。
- **上流の改善余地**: libghostty-spm へ envVars 公開の PR 提出済み（Lakr233/libghostty-spm#32）。マージ後は Swift 版も config ワークアラウンドを env 直注入へ置き換え可能。

## 8. セキュリティ

- ソケット/パイプは**同一ユーザーのみ**アクセス可能にする（AF_UNIX: 0600 + 親ディレクトリ 0700）。
- フォワーダ→本体の方向にのみデータが流れる（本体からの応答なし）。イベントは状態表示と resume 対応付けにのみ使い、受信データからのコマンド実行は行わない。
- socketPath はセッション UUID 由来で予測可能だが、書き込めるのは同一ユーザーのみであり、不正イベントの影響は表示の誤り（最悪でも誤った resume 候補の記録）に限定される。

## 9. 互換性・バージョニング方針

- フィールド追加は後方互換（受信側は未知フィールドを無視、送信側は既知フィールドを欠落させない）。
- `hook_event_name` の新値は「未知 = 破棄」により安全に無視される。
- フレーミング（1 接続 = 1 イベント）の変更は破壊的変更とみなし、socketPath の命名変更を伴うこと。
