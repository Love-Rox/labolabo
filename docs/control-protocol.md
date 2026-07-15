# LaboLabo コントロールプロトコル仕様

- **Status**: v1（2026-07-15。Rust 新版 (`labolabo-core`/`labolabo-app`) の実装を仕様として固定したもの）
- **目的**: `labolabo` CLI からタブ・作業（タスク）を操作できるようにする（cmux の claude-team 相当）。第一級ユースケースは、LaboLabo 内で動く Claude セッションが `labolabo tab open --title reviewer -- claude ...` で**自分の作業内に新タブとしてチームメイトを起動する**こと。人間・エージェント・スクリプトのいずれからも同じ操作面を使う。
- **背景**: `plans/012-task-model-and-control-cli.md` §2（コマンド案・env 文脈解決・不変条件・セキュリティ境界の初期スケッチ）。本書がその実装仕様としての正である。この wave で `focus` のサブコマンド形が `--task`/`--pane` フラグ形に確定した（plans/012 の `labolabo focus <task|tab id>` という古い位置引数スケッチを本書が上書きする）。
- **hooks プロトコル（`docs/hooks-protocol.md`）との関係**: hooks は「エージェント → 本体」への**受信専用**（fire-and-forget）チャネル。コントロールプロトコルは CLI/エージェントから本体を操作する**双方向 RPC**（1 リクエスト = 1 レスポンス）で、**別チャネル**（別ソケット）とする。実装は `hooks`/`hook_settings` と対になる形で `control`/`control_protocol` に分けてある（後者が純ロジック、前者が AF_UNIX トランスポート）。

## 1. 目的（確定）

- `labolabo` CLI からタブ・作業（タスク）を操作できるようにする。
- LaboLabo 内の Claude セッションが、自分の作業内にサブエージェント用のタブを開けるようにする。
- 人間・エージェント・スクリプトが同一の操作面（同一ワイヤプロトコル）を使う。

## 2. 不変条件（確定）

- 実行チャネルは**同一ユーザーのみ**アクセス可能（AF_UNIX ソケット 0600 + 親ディレクトリ 0700 / Windows は同等の ACL — 未実装、§9 参照）。
- **不可視実行の禁止**: `tab open` で起動されるプロセスは**常にユーザーに見えるタブの中**で実行される。アプリはバックグラウンドでコマンドを実行して結果だけ返す、という API は提供しない。
- **自動探索なし**: CLI はソケットパスを推測・列挙しない（§4）。複数の LaboLabo インスタンスが同時に動いている環境で、誤ったインスタンスへコマンドが配線されるのを防ぐ。

## 3. トランスポート（確定）

- **チャネル**: AF_UNIX（`SOCK_STREAM`）。hooks ソケットとは**別ソケット**（別ファイル）。アプリインスタンスごとに 1 本。
  - Windows（Named Pipe）は本書の§9で章のみ予約し、未実装（docs/hooks-protocol.md §4 と同じ立て付け）。
- **socketPath**: `/tmp/labolabo/control-<アプリ起動時に生成する UUID の先頭 10 文字（ハイフン除去・小文字）>.sock`
  - hooks ソケット（`/tmp/labolabo/<10hex>.sock`、docs/hooks-protocol.md §4）とは別の UUID から生成される、別ファイル。`control-` プレフィクスで両者を視覚的・字句的に区別する。
  - ディレクトリ `/tmp/labolabo` は 0700 で作成（hooks と共有）。ソケットは bind 後に **0600** へ chmod。
  - アプリ起動のたびに新しい UUID から生成される（hooks ソケットと同じく、起動時に残骸を unlink してから bind）。
- **フレーミング**: **1 接続 = 1 リクエスト = 1 レスポンス**。
  1. クライアントが接続し、リクエスト JSON の全バイトを書き込む。
  2. クライアントは書き込み側を **half-close**（`shutdown(SHUT_WR)`）する。長さプレフィクスや改行終端は使わない — サーバー側の「EOF まで読む」が「リクエストの終わり」を意味する（hooks の「1 接続 = 1 イベント、EOF まで読む」と同じ考え方を、双方向に拡張したもの）。
  3. サーバーはリクエストを EOF まで読み切り、ハンドラを呼び、レスポンス JSON の全バイトを書き込んでからソケットを close する。
  4. クライアントはレスポンスを EOF まで読み切る。
  - 要するに「書いて half-close → 読む」（クライアント）/「読み切ってから書いて close」（サーバー）。
- **受信側の実装**: `labolabo_core::control::ControlServer`（`crate::hooks::UnixSocketEventTransport` と同型の bind/chmod/accept ループ/`stop()`）。1 接続ずつ順番に処理する（同時実行なし — 制御コマンドは低頻度なので、hooks の accept ループと同じ単純さを優先した）。ハンドラは同期関数 `Fn(Vec<u8>) -> Vec<u8>`（生バイト列 in/out）で、実際のコマンドディスパッチ（gpui メインスレッドでの App 状態変更）は `labolabo-app` 側がこのハンドラの中で行う（§7）。

## 4. コンテキスト解決（確定）

### 4.1 ソケットの発見

CLI がどのソケットに繋ぐかは次の優先順で解決する（**自動探索はしない**）:

1. `--socket <path>` フラグ
2. 環境変数 `LABOLABO_CONTROL_SOCKET`
3. どちらもなければエラー（終了コード 2、§8）

`LABOLABO_CONTROL_SOCKET` は、LaboLabo が生成した端末ペインで起動されるすべてのプロセスに、既存の `LABOLABO_PANE`/`LABOLABO_TASK`（docs/hooks-protocol.md §7）と同じ場所（PTY spawn 時の env ブロック）で注入される。

### 4.2 「現在の作業」の解決（`--task current` / 省略時）

- LaboLabo 配下の端末で実行された CLI は、環境変数 `LABOLABO_TASK` から「現在の作業」を解決できる。
- **リクエスト全体への付与**: CLI は起動時の環境変数 `LABOLABO_TASK`/`LABOLABO_PANE`（空文字は「未設定」と同じ扱い）を読み、リクエストのトップレベル `labolabo_task_id`/`labolabo_pane_id` フィールドに常に載せる（hooks の `annotate_ids`（docs/hooks-protocol.md §3.2/§7）と同じ「フォワーダ/CLI が env を読んでリクエストに注釈する」パターン）。これはコマンドの種類やフラグの有無に関わらず行う。
- **`--task` フラグの意味**（`tab open`/`tab list` — `focus` は §5.4 で別扱い）:
  - `--task <具体的な ID>` を渡した場合: リクエストの `params.task` にその ID をそのまま載せる（明示指定が最優先）。
  - `--task current` を渡した場合、または `--task` を**省略**した場合: `params.task` は省略する（`null`）。サーバー側はこれを「トップレベルの `labolabo_task_id` を使え」という意味に解釈する（§4.3）。
  - 空文字列 `--task ""` は「省略」と同じ扱い。
- この 2 段構え（CLI が `--task current`/省略を「アンビエントを使え」という空値に正規化し、サーバーがアンビエントへフォールバックする）により、`labolabo tab open --title reviewer -- claude ...`（`--task` を一切書かない、plans/012 が挙げるフラグシップの使い方）が「自分がいま動いている作業」に新タブを開く、という意味になる。

### 4.3 サーバー側の最終解決

サーバーは次の優先順で対象タスク ID を決める:

1. `params.task`（明示指定。§4.2 で説明した通り、`current`/省略はここに来ない）
2. リクエストの `labolabo_task_id`（アンビエントコンテキスト）
3. どちらもなければエラー `"no task context: run this from inside a LaboLabo-spawned pane, or pass --task <id>"`（そのまま `ok:false` のレスポンスとして返す。ソケット自体には繋がっているので接続失敗ではない — 終了コード 1）

## 5. コマンド（v1 スコープ、確定）

コマンド名は `command` フィールドの文字列値（snake_case）。

### 5.1 `tab_open`

CLI: `labolabo tab open [--task <id|current>] [--title <title>] [-- <command...>]`

指定タスクの**フォーカスされているペイン（タブ群）**に新しいタブを開く。`-- <command...>` があればそのタブでそのコマンドを実行し（各引数をシェルクォートして `sh -c` へ渡す — `labolabo_core::shell_quote` と同じ規則）、なければデフォルトシェルを起動する。タスクの workspace がまだロードされていなければこの呼び出しでロードする（＝そのタスクを初めて操作する最初のペインもこの経路で生成される）。

- 対象タスクの workspace/ワークツリー/エージェント（実行中の Claude セッション等）には一切影響しない — 既存のペインを止めたり作り直したりしない、純粋な追加操作。
- **不変条件の実現**: 新しいタブは既存のタイル/タブ木に追加され、通常のタブと同じ描画経路（`task_workspace::render_tile`）で見えるようになる。バックグラウンドで実行されて結果だけ返す、という経路は存在しない。

`params`:

| フィールド | 必須 | 型 | 意味 |
|---|---|---|---|
| `task` | – | string | §4.2 参照。省略/`current` は `null` |
| `title` | – | string | 新タブのタイトル。省略時は `PaneKind::Terminal` の既定タイトル |
| `command` | – | string[] | 実行するコマンドの argv。省略/空配列はデフォルトシェル |

成功時 `result`: `{"task_id": "<uuid>", "pane_id": "<uuid>"}`

- `pane_id` は hooks 経路が既に使っている「ペインの外部安定 ID」（`LABOLABO_PANE` に注入される UUID、docs/hooks-protocol.md §7）と同じ値・同じ名前空間。新しく発行されたこの ID は同じプロセスが動いている間、`focus --pane`（§5.4）や将来の `tab close` 等の対象として使える。`labolabo-core::tiling::PaneId`（プロセス内カウンタ、外部非公開）とは別物。

エラー例: 対象タスクが存在しない、`ensure_workspace_loaded` 後もフォーカスペインが取れない、PTY spawn 失敗。

### 5.2 `task_list`

CLI: `labolabo task list [--json]`

現在ロードされている（かつ `Active` な）Task の一覧を返す。`Done`/`Archived` はこの wave では一覧に出ない（そもそもロードされない — `plans/012` §1 の「done 済み Task の resume 可否」は将来課題のまま）。

`params`: なし。

成功時 `result`: `{"tasks": [{"id", "title", "kind", "repo_name", "working_directory", "status"}, ...]}`

- `kind`: `"worktree"` | `"attached"`（`TaskKind::tag()`）
- `status`: 常に `"active"`（`TaskStatus::tag()`）— 上記の理由で他の値は出ない

### 5.3 `tab_list`

CLI: `labolabo tab list [--task <id|current>] [--all] [--json]`

- `--task` を指定: そのタスクのタブだけを返す（§4.2/§4.3 の解決規則。存在しないタスク ID はエラー）。
- `--task` を省略し `--all` も付けない: アンビエントコンテキスト（`labolabo_task_id`）があればそのタスクに絞る。なければ（LaboLabo の外から実行した場合など）全タスクのタブを返す。
- `--all`: `--task`/アンビエントを無視して**すべてのロード済みタスク**のタブを返す。

`params`:

| フィールド | 必須 | 型 | 意味 |
|---|---|---|---|
| `task` | – | string | §4.2 と同じ規則 |
| `all` | – | bool | `true` なら §5.3 の `--all` 挙動 |

成功時 `result`: `{"tabs": [{"task_id", "pane_id", "title", "kind", "focused"}, ...]}`

- `pane_id` は `null` の場合がある（そのペインの `Terminal` セッションがまだ spawn されていない、または hooks ルーティング対象外の場合）。
- `kind`: `PaneKind::raw_value()`（`"terminal"` | `"files"` | `"diff"` | `"commits"`）。
- `focused`: そのタスクの workspace の `focused_pane` と一致するか。

### 5.4 `focus`

CLI: `labolabo focus --task <id>` または `labolabo focus --pane <id>`（**どちらか片方を必ず指定**。両方/どちらも無しはエラー）

- `plans/012` §2 の `labolabo focus <task|tab id>`（位置引数、型で分岐）という初期スケッチを、本書で `--task`/`--pane` の明示フラグ形に確定させた。
- **`--task`/`--pane` はここでは「現在」の意味を持たない**: `current`/省略のアンビエント解決（§4.2）は `tab_open`/`tab_list` 専用。`focus` は常に具体的な ID を要求する（「今の作業をフォーカスする」は無意味な操作のため）。
- `--task <id>`: そのタスクを選択する(`LaboLaboApp::select_task` と同じ経路 — 未ロードなら workspace をロードする)。
- `--pane <id>`: `tab_open` が返した pane_id（= `LABOLABO_PANE` の値）を解決し、そのペインが属するタスクを選択したうえで、そのペインをタブとして選択する。未知の pane_id はエラー。

`params`:

| フィールド | 必須 | 型 | 意味 |
|---|---|---|---|
| `task` | 片方必須 | string | 対象タスク ID |
| `pane` | 片方必須 | string | 対象ペイン ID（`LABOLABO_PANE` 値） |

成功時 `result`: `{"task_id": "<uuid>"}`（`--pane` 指定時は `{"task_id": "<uuid>", "pane_id": "<uuid>"}`）

### 5.5 予約（未実装）

- `task new --repo <path> [--branch <name>] [--attached] [--title <t>] [--command <cmd>]` — `plans/012` §2 のコマンド案にある通り、UI の「新しい作業」フローと重なるため**このwaveではスコープ外**。将来章として名前のみ予約する。
- MCP サーバとしての同 RPC 公開（`plans/012` §2 の「将来オプション」）。CLI が先、MCP は後。

## 6. リクエスト/レスポンス JSON スキーマ（確定）

### リクエスト

```json
{
  "command": "tab_open",
  "params": { "task": null, "title": "reviewer", "command": ["claude"] },
  "labolabo_task_id": "5b6c...-task-uuid",
  "labolabo_pane_id": "a1b2...-pane-uuid"
}
```

| フィールド | 必須 | 型 | 意味 |
|---|---|---|---|
| `command` | ✔ | string | §5 のコマンド名 |
| `params` | – | object | コマンド別パラメータ（省略時は `{}` と同じ扱い） |
| `labolabo_task_id` | – | string \| null | CLI 実行時の env `LABOLABO_TASK`（§4.2） |
| `labolabo_pane_id` | – | string \| null | CLI 実行時の env `LABOLABO_PANE`（現状どのコマンドも参照しないが、将来のコマンド／監査用に常に載せる） |

### レスポンス

成功:

```json
{ "ok": true, "result": { "...": "..." } }
```

失敗:

```json
{ "ok": false, "error": "human-readable message" }
```

- `result`/`error` はどちらか一方のみ存在する（`ok` の値と対応）。
- パースできないリクエスト（不正 JSON、`command` 欠落など）も **`ok:false` のレスポンスを返す**（接続自体は成功しているため）。これは hooks の「不正イベントは黙って破棄」（docs/hooks-protocol.md §5）とは異なる — control は同期的にクライアントが応答を待っているので、黙殺せず理由を返す。

## 7. アプリ側の実行（確定）

- サーバーの accept ループは hooks の `HookRuntime`/`AgentStatusBus` と同様、専用スレッドに常駐する（`labolabo_core::control::ControlServer`）。
- 実際のコマンド実行（Task/タブの状態変更）は **gpui メインスレッドでの `App`/`Window` 状態変更**として行う。accept スレッドはハンドラ内で `std::sync::mpsc`（同期チャネル）越しにメインスレッドへリクエストを渡し、返信を待ってから（タイムアウト付き）レスポンスを書き込む — `labolabo-app::control::ControlRuntime`/`spawn_control_bridge` を参照。
- `tab open` はタブ生成の**既存の経路**（UI の「+」ボタンと同じ `LaboLaboApp::open_tab_for_control`／内部で `spawn_runtime_for_task` を呼ぶ）を再利用する。env 注入（`LABOLABO_PANE`/`LABOLABO_TASK`/`LABOLABO_CONTROL_SOCKET`）・hooks ルーティングテーブルへの登録・レイアウトの永続化は、UI 操作で新しいタブを開いたときと完全に同じコードパスを通る。

## 8. CLI（確定）

バイナリ名 `labolabo`（`labolabo-app` パッケージの追加 `[[bin]]`）。

```
labolabo [--socket <path>] tab open [--task <id|current>] [--title <t>] [--json] [-- <command...>]
labolabo [--socket <path>] task list [--json]
labolabo [--socket <path>] tab list [--task <id|current>] [--all] [--json]
labolabo [--socket <path>] focus --task <id> [--json]
labolabo [--socket <path>] focus --pane <id> [--json]
```

- 引数パースは手書き（依存追加なし）。コマンド数・フラグ数が少なく（4 コマンド、フラグは高々 3 つ）、`clap` 相当の機能（自動 usage 生成、サブコマンドツリー、補完）を必要とするほどの複雑さがないため。バイナリサイズ・ビルド時間も、頻繁に（エージェントが毎回のタブ起動で）呼び出す小さな CLI としては手書きの方が有利。
- **終了コード**:
  - `0`: 成功（`ok:true`）
  - `2`: 接続失敗 — ソケットパスが解決できない（§4.1 のフォールバックが尽きた）、`connect(2)` 自体が失敗、または CLI 側の引数パースエラー（サーバーに到達する前に失敗するという点で「接続失敗」と同じバケツに含めた。本書がこの解釈を明記する — 3 種類の終了コードしか規定されていないため）
  - `1`: アプリ側エラー — 接続には成功したがレスポンスが `ok:false`
- **出力**:
  - 既定: 人間向けの簡潔なテキスト（例: `tab_open` 成功時は `opened pane <pane_id> in task <task_id>`）。エラーは `error: <message>` として stderr に出す。
  - `--json`: サーバーから受け取った生のレスポンス JSON をそのまま stdout に出す（成功・失敗どちらも）。

## 9. エラー・互換性・セキュリティ（確定）

- **互換性方針**: docs/hooks-protocol.md §9 と同じ。フィールド追加は後方互換(受信側は未知フィールドを無視、送信側は既知フィールドを欠落させない)。未知の `command` 値は `ok:false` のエラーレスポンスで安全に拒否される(hooks の「未知イベントは黙って破棄」とは異なり、こちらは明示的に伝える — §6)。フレーミング(1 接続 = 1 リクエスト = 1 レスポンス)の変更は破壊的変更とみなし、socketPath の命名規則変更を伴うこと。
- **セキュリティ**: ソケットは同一ユーザーのみアクセス可能(AF_UNIX 0600 + 親ディレクトリ 0700)。`tab open` は任意コマンド実行に等しいため、ソケットに書き込めること自体がその LaboLabo インスタンスの全操作権限を意味する(同一ユーザーの別プロセス／別エージェントも含む)。これは hooks ソケットと同じ信頼境界(「同一ユーザーの誰でも」)であり、`plans/012` §2 の「セキュリティ境界」がそのまま適用される。
- **Windows(未実装・予約)**: Named Pipe を想定(docs/hooks-protocol.md §4 の同種の予約と同じ立て付け)。フレーミングの意味論(1 接続 = 1 リクエスト = 1 レスポンス、書いて half-close → 読む)を保てばトランスポートは差し替え可能。
