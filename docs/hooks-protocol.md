# LaboLabo hooks ワイヤプロトコル仕様

- **Status**: v1.2（2026-07-17。v1 = 2026-07-14 に実装済みの挙動を仕様として固定。v1.1 で §4.2 の Windows Named Pipe トランスポートを「予約」から実仕様へ昇格。v1.2 で §5 に `source` フィールドを追加し、`SessionStart`/`source: "compact"` の特別扱いを明文化 — 「実機バグの根本原因調査」波での修正）
- **目的**: Claude Code の hooks を使ったエージェント状態検出・セッション対応付けの配線を、**実装言語・OS に依存しない形**で再実装可能にする。Rust 版（クロスプラットフォーム化）はこの文書を正として実装する。
- **実装の現在地（Swift 版）**: 解釈層 = `AgentEventParser`、トランスポート契約 = `AgentEventTransport`、AF_UNIX 実装 = `UnixSocketEventTransport`（いずれも `Sources/LaboLaboEngine/Agent/`）。フォワーダ = `app/Sources/HookForwarder.swift`。hooks 注入 = `app/Sources/AgentSessionModel.swift`。
- **実装の現在地（Rust 版）**: 解釈層 = `agent_event_parser`、トランスポート契約 = `hooks::AgentEventTransport`、AF_UNIX 実装 = `hooks::UnixSocketEventTransport`、Named Pipe 実装（§4.2）= `hooks::NamedPipeEventTransport`、フォワーダ = `labolabo-hook` bin + `hooks::forward_hook`（いずれも `rust/crates/labolabo-core/`）。

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
- **既知の制限（注入スコープ）**: 注入されるのは LaboLabo に登録済みの Task/セッションの worktree ディレクトリのみ。そのディレクトリの**外**（別リポジトリ、Task 化していない任意のディレクトリ等）で起動された `claude` は、LaboLabo が生成したペイン内であっても `settings.local.json` に hook が無いため一切イベントを送らない（インジケーターは最後に観測した状態のまま更新されなくなる。新規なら「none」のまま）。§7 の `LABOLABO_PANE` 注入（ペインの環境変数）とは独立した制約であり、環境変数だけでは解決できない — 修正するには任意ディレクトリへの hooks 注入（グローバル `~/.claude/settings.json` 相当）が必要になり、LaboLabo を使わない他の Claude Code 利用にも影響するため対応していない。

## 3. フォワーダ契約（`labolabo --hook <socketPath>`）

1. stdin を **EOF まで**全読みする（Claude Code が hook の stdin にイベント JSON を渡す）。
2. **ペイン ID の付与**: 環境変数 `LABOLABO_PANE` が非空で、かつ stdin が JSON オブジェクトとして解釈できる場合、トップレベルに `"labolabo_pane_id": "<値>"` を追加して再シリアライズする。変数が無い・JSON でない場合は**原文のまま**転送する。
   - フォワーダは hook として Claude の子孫プロセスで走るため、ペインのシェルに注入された環境変数を継承している（§7）。
3. `<socketPath>` へ connect → 全バイトを write → close → **即 exit(0)**。接続失敗・パス過長などあらゆる失敗も exit(0)（hook の失敗で Claude を止めない）。

## 4. トランスポート（受信側）

OS 別に 2 実装（§4.1 / §4.2）。共通の契約:

- **フレーミング**: **1 接続 = 1 イベント**。受信側は accept 後 EOF（相当）まで読み、その全体を 1 メッセージとして扱う。長さプレフィクスや区切り文字は使わない。
- **チャネル名**: セッション UUID から導出した先頭 10 文字（ハイフン除去・小文字。以下「10hex」）を含む、セッションごとに 1 本のチャネル。§2 の `<socketPath>` にはこのチャネル名がそのまま入る（フォワーダはどの OS でも「渡された文字列へ connect する」だけで、パスとパイプ名を区別しない）。
- **注意（既知のレース）**: アプリ再起動直後、旧プロセス由来の遅延イベント（死んだ claude の SessionEnd 等）が再 bind 済みの同一チャネル名へ届くことがある。消費側はこれを前提に防御する（§6）。

### 4.1 AF_UNIX（macOS / Linux）

- **チャネル**: AF_UNIX ソケット（SOCK_STREAM）。
- **socketPath**: `/tmp/labolabo/<10hex>.sock`（Rust: `hook_settings::socket_path_from_uuid`）
  - ディレクトリ `/tmp/labolabo` は 0700 で作成。ソケットは bind 後に **0600** へ chmod（同一ユーザーのみ）。
  - 同一パスはアプリ再起動を跨いで再利用される（起動時に残骸を unlink してから bind）。
- **EOF**: クライアントの close（または書き込み側 shutdown）がそのまま EOF。

### 4.2 Named Pipe（Windows）

Rust 版で実装済み（`hooks::NamedPipeEventTransport`）。v1 で挙げた候補（AF_UNIX for Windows / loopback TCP）ではなく Named Pipe を採用した — 追加のポート管理・ファイアウォール考慮が不要で、DACL による同一ユーザー制限が第一級でできるため。

- **チャネル**: セッションごとに 1 本の Named Pipe。**byte mode**・**inbound**（サーバ=受信専用、クライアント=送信専用）。
- **パイプ名**: `\\.\pipe\labolabo-<10hex>`（Rust: `hook_settings::hook_pipe_name_from_uuid`）。§2 の `<socketPath>` にはこのパイプ名が入る。
- **アクセス制御**: パイプ作成時に DACL = 「現在ユーザー + SYSTEM に GENERIC_ALL、その他は拒否（protected）」を明示指定する（0600 + 0700 相当。§8）。既定 DACL は Everyone に read を許すため使わない。DACL を構築できない場合は**バインドせず失敗する**（fail closed）。
- **EOF 相当**: クライアントは全バイトを write → **FlushFileBuffers**（未 flush のまま CloseHandle するとパイプの残バイトが破棄されうるため）→ CloseHandle。サーバ側はこの切断（`ERROR_BROKEN_PIPE` / `ERROR_PIPE_NOT_CONNECTED`）を **EOF として扱い**、それまでに読めた全体を 1 イベントとする — §4.1 の「EOF まで読む」と観測上同一。
- **残骸処理**: パイプ名は最終ハンドルの close と同時に消滅するため、§4.1 の「起動時に unlink」に相当する手順は不要（同名パイプの再利用はアプリ再起動でそのまま成立する）。
- **§2 のコマンド文字列**: `'<binary>' --hook '<socketPath>'` のシングルクォート形は POSIX sh 前提。Windows でネイティブに hooks を注入する際のクォート規則（cmd / PowerShell）は Rust アプリ側 Windows 対応波で確定する（本書の予約事項）。バイナリが argv で `--hook <パイプ名>` を受け取る契約自体は同一。

## 5. イベント JSON

受信側が解釈するトップレベルフィールド（**すべて文字列**。他のフィールドは無視 = 前方互換）:

| フィールド | 必須 | 意味 |
|---|---|---|
| `hook_event_name` | ✔ | Claude Code の hook イベント名。未知の値は**イベントごと破棄** |
| `source` | – | `SessionStart` の発火要因: `startup` / `resume` / `clear` / `compact`（Claude Code hooks リファレンス）。`SessionStart` 以外でも受信側はフィールドとして拾う（§9 の前方互換方針の裏返し）が、意味を持つのは `SessionStart` のみ |
| `session_id` | – | Claude セッション ID（`claude --resume` に使う） |
| `transcript_path` | – | transcript(JSONL) の絶対パス |
| `cwd` | – | イベント発生時の作業ディレクトリ |
| `labolabo_pane_id` | – | フォワーダが付与した端末ペイン UUID（§3）。外部ターミナル起動分には付かない |

### イベント → 状態のマッピング（正: `AgentStatus.from(hookEvent:)`）

| hook_event_name | source | AgentStatus |
|---|---|---|
| `SessionStart` | `startup` / `resume` / `clear` / 欠落・未知 | `starting` |
| `SessionStart` | `compact` | `running`（**注**下記） |
| `UserPromptSubmit` / `PreToolUse` / `PostToolUse` | – | `running` |
| `Notification` | – | `waitingForInput`（入力・許可待ち） |
| `Stop` / `SubagentStop` | – | `idle`（応答完了・待機） |
| `SessionEnd` | – | `ended` |
| それ以外 | – | （破棄） |

**注（`SessionStart`/`source: "compact"`）**: 自動コンパクション（および `/compact` 手動実行）は会話の**途中**で `SessionStart` を発火させる — セッションの開始ではない。これを他の `source` と同様に `starting` へ写像すると、コンパクション直前まで `running`（緑）だった表示が `starting`（橙）へ後退し、実際には作業が継続中なのに「動作中でも緑にならない」ように見える（実機バグ報告の根本原因の一つ、第13波 a で特定）。そのため `source: "compact"` だけは `running` を維持する特別扱いとする。`startup`/`resume`/`clear`、および `source` 欠落・未知の場合は従来どおり `starting`。

**既知の Swift/Rust 挙動差異**: この `source: "compact"` 特別扱いは **Rust 版のみ**（`labolabo_core::agent_status::AgentStatus::from_hook_event`）。Swift 版の `Sources/LaboLaboEngine/Agent/AgentStatus.swift`（`AgentStatus.from(hookEvent:)`）は `source` を一切参照せず、コンパクション後も無条件に `.starting` を返す — 同じ表示退行が残ったまま（第13波 a はこの調査でSwift側の同型バグを確認したが、修正はRust版のみをスコープとした）。Swift 側への同様の修正は別途の課題として残る。

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

- ソケット/パイプは**同一ユーザーのみ**アクセス可能にする（AF_UNIX: 0600 + 親ディレクトリ 0700 / Named Pipe: 現在ユーザー + SYSTEM のみの protected DACL — §4.2）。
- フォワーダ→本体の方向にのみデータが流れる（本体からの応答なし）。イベントは状態表示と resume 対応付けにのみ使い、受信データからのコマンド実行は行わない。
- socketPath はセッション UUID 由来で予測可能だが、書き込めるのは同一ユーザーのみであり、不正イベントの影響は表示の誤り（最悪でも誤った resume 候補の記録）に限定される。

## 9. 互換性・バージョニング方針

- フィールド追加は後方互換（受信側は未知フィールドを無視、送信側は既知フィールドを欠落させない）。
- `hook_event_name` の新値は「未知 = 破棄」により安全に無視される。
- フレーミング（1 接続 = 1 イベント）の変更は破壊的変更とみなし、socketPath の命名変更を伴うこと。
