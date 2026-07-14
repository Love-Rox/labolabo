# LaboLabo コントロールプロトコル仕様（骨子）

- **Status**: 骨子のみ（章立ての予約）。中身は Rust 新版の設計時に確定する。背景・要件は plans/012（作業ツリー内ドキュメント）を参照。
- **hooks プロトコル（docs/hooks-protocol.md）との関係**: hooks は「エージェント → 本体」への**受信専用**チャネル。コントロールプロトコルは CLI/エージェントから本体を操作する**双方向 RPC** で、**別チャネル**とする。

## 1. 目的（確定）

- `labolabo` CLI からタブ・作業（タスク）を操作できるようにする（cmux の claude-team 相当）。
- LaboLabo 内の Claude セッションが、自分の作業内にサブエージェント用のタブを開けるようにする。
- 人間・エージェント・スクリプトが同一の操作面を使う。

## 2. 不変条件（確定）

- 実行チャネルは**同一ユーザーのみ**アクセス可能（AF_UNIX 0600 / Windows は同等の ACL）。
- `tab open` 系で起動されるプロセスは**常にユーザーに見えるタブの中**で実行される（不可視実行はさせない）。

## 3. トランスポート（未確定 — 設計時に決定)

- 候補: hooks とは別の control ソケット（Unix: AF_UNIX / Windows: Named Pipe）。
- フレーミング・リクエスト/レスポンスの形式（JSON Lines / length-prefixed 等）はここで定義する。

## 4. コンテキスト解決（方針のみ）

- LaboLabo 配下の端末で実行された CLI は、環境変数 `LABOLABO_TASK` / `LABOLABO_PANE` から「現在の作業/タブ」を解決する。

## 5. コマンド（案 — 設計時に確定）

```
labolabo task new --repo <path> [--branch <name>] [--attached] [--title <t>] [--command <cmd>]
labolabo tab open [--task <id|current>] [--title <t>] -- <command...>
labolabo task list / tab list [--json]
labolabo focus <task|tab id>
```

## 6. エラー・互換性（予約）

## 7. セキュリティ詳細（予約）
