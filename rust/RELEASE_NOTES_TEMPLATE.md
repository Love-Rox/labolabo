<!--
  .github/workflows/rust-release.yml が `{{VERSION}}`/`{{TAG}}` を実際の値に
  置換して draft リリースの本文として使うテンプレート。手動編集する際は
  プレースホルダの綴りを変えないこと（ワークフローの sed が文字列一致で
  置換するため）。

  pre-release フラグはワークフロー側がバージョンの `-` サフィックス
  （例 1.1.0-rc.1）の有無で自動判定する。本文はどちらでも通用する表現に
  してあるので、RC を切るときもこのテンプレートをそのまま使える。
-->

# LaboLabo-rs {{VERSION}}

Claude Code などの AI コーディングエージェントを複数の git worktree で並列に走らせ、
端末とライブな Git 差分を横に並べて確認するアプリの **Rust / クロスプラットフォーム版**（macOS / Linux / Windows）です。

> macOS で最も長く検証されています。Windows / Linux は比較的新しい対応です。
> 既知の制限は下記「既知の制限」を必ずお読みください。

## 主な機能

- **タブ + タイル分割の端末シェル**（[gpui](https://www.gpui.rs/) 製）-- 複数の作業（Task）をサイドバーから切り替え、
  1 つの Task 内でも端末タブ・タイル分割を自由に組み合わせられます。
- **ライブな Git ペイン** -- 変更ファイル一覧・Diff・コミット履歴を、端末の隣でリアルタイムに確認できます。
- **Claude Code hooks 連携** -- エージェントの状態（実行中/待機中など）をサイドバーのドットで表示し、
  タブ別にセッションを記憶して再起動時に自動 resume します。
- **セッションの永続化** -- Task とタイルレイアウトはローカル SQLite に保存され、再起動後も復元されます。
- **Swift 版からのインポート** -- 既存の Swift（macOS ネイティブ）版のセッション DB を検出し、初回起動時に自動インポートします。
- **日英 UI 切り替え**（設定 > 言語）。

## インストール

タグ: `{{TAG}}`

| OS | ダウンロード | 手順 |
|---|---|---|
| macOS | `LaboLabo-rs-{{VERSION}}.zip` | 展開して `LaboLabo-rs.app` を `/Applications` などへ。アドホック署名のため初回起動は右クリック > 開く、または `xattr -dr com.apple.quarantine LaboLabo-rs.app` が必要な場合があります。 |
| Linux | `LaboLabo-rs-linux-{{VERSION}}-<arch>.tar.gz` | 展開して同梱の `install.sh` を実行（root 不要、`~/.local` 配下にインストール）。詳細は同梱の README.md 参照。 |
| Windows | `LaboLabo-rs-windows-{{VERSION}}-<arch>.zip` | 展開して `bin\labolabo-app.exe` を実行、または同梱の `.ico` でショートカットを作成。詳細は同梱の README.md 参照。 |

いずれも署名は行っていません（macOS はアドホック署名・Developer ID なし、Linux/Windows は無署名）。
詳しい配布方針は [`rust/README.md`](https://github.com/Love-Rox/labolabo/blob/main/rust/README.md) を参照してください。

## 既知の制限

- **Linux / Windows: 実機 GUI 起動は未検証です。** CI ではビルドとヘッドレスなユニット/結合テストのみを実施しており、
  実際のデスクトップ環境での表示・操作は開発チームの手元で確認できていません（開発ループに Linux/Windows 実機がないため）。
  問題を見つけたら Issue でご報告ください。
- **macOS: 通知/自動更新チェックのみ簡易実装。** アップデート確認は起動時に一度だけバックグラウンドで行い、
  見つかった場合はサイドバーに控えめなバナーを表示します（OS 通知は出しません）。
- この Rust 版は Swift（macOS ネイティブ）版からのクロスプラットフォーム移植の途上であり、
  一部の機能（IDE で開く、Finder で表示 等）は現時点で macOS 限定です。

## フィードバック

不具合・要望は [GitHub Issues](https://github.com/Love-Rox/labolabo/issues) までお願いします。
