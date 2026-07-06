# LaboLabo

複数の AI コーディングエージェント（Claude Code / Codex / Gemini …）を **1 セッション = 1 git worktree** で並列に走らせ、**各セッションのライブ Git 状態とファイル差分を、動いているエージェント端末の真横で確認できる** macOS ネイティブ・デスクトップアプリ。

🌐 **サイト: <https://labolabo.love-rox.cc>**

## コンセプト

- **左ペイン**: リポジトリ/セッションのツリー（セッション名・ブランチ名・状態を一望）
- **中央**: 本物の GPU 端末（libghostty 埋め込み）でエージェントがインタラクティブに動く
- **右ペイン**: 変更ファイル一覧 ＋ Diff ⇄ ファイル全文の切替表示

## インストール

Homebrew（cask）で配布しています。

```sh
brew tap love-rox/tap
brew trust love-rox/tap          # 第三者 tap の信頼（Homebrew の要件）
brew install --cask labolabo
```

更新は `brew upgrade --cask labolabo`。`.app` を直接使う場合は [Releases](https://github.com/Love-Rox/labolabo/releases) から入手できます。

> アドホック署名（Apple 公証なし・無料配布）のため、初回起動は macOS の Gatekeeper がブロックします。インストール時に表示される `caveats` の手順（`xattr -dr com.apple.quarantine "/Applications/LaboLabo.app"`、または Finder で右クリック →「開く」）で許可してください。

## スタック

- ネイティブ macOS / **Swift + SwiftUI**
- 端末は **libghostty** を XCFramework として埋め込み（`scripts/build-ghostty.sh` で生成）
- エンジン層（プロセス / Git / 状態）は UI 非依存の Swift `actor` 群
- 永続化は **GRDB.swift（SQLite）**
- macOS 専用・Developer ID 署名 + notarization で配布（Mac App Store 外）

## 構成

```
Sources/
  LaboLaboEngine/   Git エンジン・エージェントアダプタ・状態バス（UI 非依存）
  LaboLaboStore/    GRDB モデル + マイグレーション
  LaboLaboUI/       SwiftUI ビュー（後続）
  LaboLaboApp/      アプリ本体（後続）
scripts/
  build-ghostty.sh  libghostty を GhosttyKit.xcframework としてビルド
vendor/
  GhosttyKit.xcframework  （ビルド成果物・git 管理外）
```

## 開発

```sh
swift test                          # エンジン層の単体テスト

# macOS アプリ（XcodeGen で .xcodeproj を生成してから Xcode/xcodebuild）
brew install xcodegen
xcodegen generate --spec app/project.yml
open app/LaboLabo.xcodeproj         # もしくは:
xcodebuild -project app/LaboLabo.xcodeproj -scheme LaboLabo -destination 'platform=macOS' build
```

`app/LaboLabo.xcodeproj` は `app/project.yml` から生成される成果物なので git 管理外。

詳細な実装計画は別途プランを参照。

## ブランチ運用

- `main`: 保護ブランチ。直接 push 禁止。`dev` からの PR のみ受け付ける。
- `dev`: 統合ブランチ。feature ブランチからの PR を受ける。
- `feature/*`: 各作業はここで。`dev` へ PR。
