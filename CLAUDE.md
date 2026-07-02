# LaboLabo — Claude / 開発ガイド

複数の AI コーディングエージェント（Claude Code / Codex / Gemini …）を **1 セッション = 1 git worktree** で並列に走らせ、**ライブな Git 状態とファイル差分を端末の真横で確認**する macOS ネイティブアプリ。

## アーキテクチャ / 構成

- **ルート = SwiftPM パッケージ**（再利用ライブラリ。UI 非依存）
  - `LaboLaboEngine`: Git（`GitEngine`/`GitRunner`、porcelain v2・unified diff パーサ、`FileWatcher`=FSEvents）
  - `LaboLaboStore`: GRDB(SQLite) 永続化（`SessionDatabase`/`SessionRecord`）
- **`app/` = macOS アプリ**（**XcodeGen** で生成）
  - `app/project.yml` → `app/LaboLabo.xcodeproj`（**生成物・git 管理外**）
  - ルート package を `..` 参照（`LaboLaboEngine`/`LaboLaboStore`）＋ プレビルト **`libghostty-spm`**(`GhosttyTerminal`)
  - SwiftUI 3 ペイン: 左=repo/session ツリー / 中=libghostty 端末（タブ複数）/ 右=WorkPane（変更ファイル一覧・Diff⇄全文）

## ビルド / テスト

```sh
# エンジン/ストアの単体テスト（CI もこれ）
swift test

# アプリ（.xcodeproj を生成してから）
brew install xcodegen
xcodegen generate --spec app/project.yml      # ★ ファイルを追加/削除したら必ず再生成
xcodebuild -project app/LaboLabo.xcodeproj -scheme LaboLabo \
  -destination 'platform=macOS,arch=arm64' -skipMacroValidation \
  CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO build
open ~/Library/Developer/Xcode/DerivedData/LaboLabo-*/Build/Products/Debug/LaboLabo.app
# 開発時は: open app/LaboLabo.xcodeproj → ⌘R
```

> **`-skipMacroValidation` が必須**: TCA（The Composable Architecture）等の Swift マクロを
> 使うため。CLI ビルドではこのフラグ、Xcode GUI では初回に「Trust & Enable」を求められる。

## ブランチ / PR 運用

- `main`: 保護（直 push 禁止・force/削除禁止・linear・required checks=`swift test`,`enforce-dev-only`）。**`dev` からの PR のみ**受け付ける（`release-please--*` は例外）。
- `dev`: 統合ブランチ。
- 作業は **`feature/*` → dev へ PR → （まとめて）dev → main**。
- PR は **draft で起票**して内容確認後に Ready。

## バージョン / リリース

- **Conventional Commits → release-please** が version/CHANGELOG/タグを自動化。
  - version 源: `Config/Version.xcconfig` の `MARKETING_VERSION`（`// x-release-please-version` 行、手編集しない）。
  - ビルド番号(`CFBundleVersion`): `git rev-list --count HEAD`（archive 時注入）。
- コミットメッセージは Conventional Commits（`feat:`/`fix:`/`chore:` …、本文は日本語可）。

## 方針メモ

- **macOS 専用**。任意 CLI 起動・任意リポジトリアクセスのため **App Sandbox は無効**（Developer ID 署名 + notarization で MAS 外配布の想定）。
- 端末は **本物の libghostty**（プレビルト `libghostty-spm`。将来 source ビルド=ghostty v1.3.1/Zig 0.15.2 に移行予定）。
- **Swift 6 言語モード**（strict concurrency）。エンジン/ストア（Package）とアプリ（`SWIFT_VERSION: 6.0`）ともに移行済み。UI 層の TCA 化は継続課題（#16）。
- エンジン層は UI フレームワーク非依存の `actor`/プレーン型に保ち、UI（SwiftUI/将来 TCA）から切り離す。

## 設計の背景

詳しい設計判断（libghostty 埋め込み方式、状態検出=Claude hooks、cmux 方式の再起動復元 など）は別途プランに記載。Phase 0（端末＋ライブ差分＋セッション永続化）が動作する段階。
