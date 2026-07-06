# Changelog

## [0.5.0](https://github.com/Love-Rox/labolabo/compare/v0.4.1...v0.5.0) (2026-07-06)


### 新機能

* Web ランディングのデザイン言語をアプリ UI に移植 ([#70](https://github.com/Love-Rox/labolabo/issues/70)) ([8643645](https://github.com/Love-Rox/labolabo/commit/86436455d2fbe4f46ef01791d2d89bc377bb1069))


### ドキュメント

* README にインストール手順（Homebrew）とサイトリンクを追加 ([#67](https://github.com/Love-Rox/labolabo/issues/67)) ([104ed3b](https://github.com/Love-Rox/labolabo/commit/104ed3b5c7e8c9bcfcd22c940975176f50724b46))

## [0.4.1](https://github.com/Love-Rox/labolabo/compare/v0.4.0...v0.4.1) (2026-07-05)


### バグ修正

* ライトモードのアイコンのライムロゴにうっすらシャドウ（真後ろ・広め）を追加 ([#65](https://github.com/Love-Rox/labolabo/issues/65)) ([08eeef6](https://github.com/Love-Rox/labolabo/commit/08eeef646fc4473f78f1c864a05e605b754197f6))

## [0.4.0](https://github.com/Love-Rox/labolabo/compare/v0.3.2...v0.4.0) (2026-07-05)


### 新機能

* アップデート確認を設定に追加（GitHub Releases・無料配布路線の第一歩） ([#50](https://github.com/Love-Rox/labolabo/issues/50)) ([12028bd](https://github.com/Love-Rox/labolabo/commit/12028bd72d2ed032102e5266793d3a677d838f23))
* バグ報告画面（環境情報つき GitHub Issue 作成）を追加 ([#57](https://github.com/Love-Rox/labolabo/issues/57)) ([656782a](https://github.com/Love-Rox/labolabo/commit/656782ac8499f1a99832929c05498ccf59451dc2))
* メイン上部に org 付きリポジトリ名＋色表示／サイドバー選択を指定色に ([#51](https://github.com/Love-Rox/labolabo/issues/51)) ([1b92644](https://github.com/Love-Rox/labolabo/commit/1b92644d9f8908d1bcc0addff125e6d7385d4a43))
* 平文 String の日本語 UI 文言を String(localized:) でローカライズ対応 ([#54](https://github.com/Love-Rox/labolabo/issues/54)) ([d5d5eb7](https://github.com/Love-Rox/labolabo/commit/d5d5eb7c4d22c2200730db37f40a1636f33fe74a))
* 日英ローカライズ（String Catalog・SwiftUI リテラル 110 件を英語化） ([#53](https://github.com/Love-Rox/labolabo/issues/53)) ([aa1fcd0](https://github.com/Love-Rox/labolabo/commit/aa1fcd0ebbad8f41c512d412f3a4ed7f74129dce))


### バグ修正

* About/設定に実ビルド番号を表示（CFBundleVersion に git コミット数を注入） ([#56](https://github.com/Love-Rox/labolabo/issues/56)) ([2501464](https://github.com/Love-Rox/labolabo/commit/25014643d833db103c7b06c8d9a1f15137c4f7b3))
* メニューを開くとクラッシュする（toggleSidebar: の無限再帰）を解消 ([#52](https://github.com/Love-Rox/labolabo/issues/52)) ([f38f6b4](https://github.com/Love-Rox/labolabo/commit/f38f6b42b6a8a2c37c7a6ccdc2942ce418ba5111))
* メニューを開くと落ちる/固まる macOS 26 バグを根治＋メニューバー整備 ([#58](https://github.com/Love-Rox/labolabo/issues/58)) ([1b7e89d](https://github.com/Love-Rox/labolabo/commit/1b7e89d1ba975fa23df76b1f20d3164018a32c4a))

## [0.3.2](https://github.com/Love-Rox/labolabo/compare/v0.3.1...v0.3.2) (2026-07-04)


### バグ修正

* エラー整形を共有ヘルパに集約し worktree 削除の空エラーを解消＋release-please コメント訂正 ([#46](https://github.com/Love-Rox/labolabo/issues/46)) ([df45188](https://github.com/Love-Rox/labolabo/commit/df451887684f6aeb9909d22cd915256d6facfbe4))
* 小さいウィンドウ/ネスト分割でペインが崩れる（min&gt;max）を解消 ([#47](https://github.com/Love-Rox/labolabo/issues/47)) ([c165b5e](https://github.com/Love-Rox/labolabo/commit/c165b5e39f29334140b7d3e7dfd0266facf20604))

## [0.3.1](https://github.com/Love-Rox/labolabo/compare/v0.3.0...v0.3.1) (2026-07-02)


### バグ修正

* 分割ペインの制約・比率保存が無効だった NSSplitViewDelegate セレクタ誤り＋New Session の空エラー ([#42](https://github.com/Love-Rox/labolabo/issues/42)) ([93281ed](https://github.com/Love-Rox/labolabo/commit/93281ed36e4f7f2bd272b02f5be51cce07e70bbc))

## [0.3.0](https://github.com/Love-Rox/labolabo/compare/v0.2.0...v0.3.0) (2026-07-02)


### 新機能

* PR 作成フロー（push→gh pr create）とツール診断 doctor ([#33](https://github.com/Love-Rox/labolabo/issues/33)) ([0b8b22d](https://github.com/Love-Rox/labolabo/commit/0b8b22d752242a0dcb37a3c0c0527119ae3cab82)), closes [#14](https://github.com/Love-Rox/labolabo/issues/14) [#15](https://github.com/Love-Rox/labolabo/issues/15)
* エージェントアダプタ抽象（Claude/Codex/Gemini）と能力ベースの UI 出し分け ([#36](https://github.com/Love-Rox/labolabo/issues/36)) ([5a36ef0](https://github.com/Love-Rox/labolabo/commit/5a36ef03343c105127049f30615a7e4fa7f4850c)), closes [#17](https://github.com/Love-Rox/labolabo/issues/17)
* セッション間の変更ファイル逆引きとコンフリクト警告 ([#37](https://github.com/Love-Rox/labolabo/issues/37)) ([0022454](https://github.com/Love-Rox/labolabo/commit/0022454a9372715c5f7a5da86dac0e7af95b42ca)), closes [#18](https://github.com/Love-Rox/labolabo/issues/18)
* 使用量/コストの推定表示（transcript から集計） ([#38](https://github.com/Love-Rox/labolabo/issues/38)) ([2ccec0a](https://github.com/Love-Rox/labolabo/commit/2ccec0af82080760a7331873203304103d55483e)), closes [#19](https://github.com/Love-Rox/labolabo/issues/19)


### バグ修正

* ツール解決を ToolLocator に一元化し doctor 判定と実行を一致させる ([#34](https://github.com/Love-Rox/labolabo/issues/34)) ([dba0351](https://github.com/Love-Rox/labolabo/commit/dba035105ae16ebbbd4a599ae5592b4ed7413032))


### リファクタリング

* Swift 6 言語モードへ移行（エンジン/ストア/アプリ） ([#35](https://github.com/Love-Rox/labolabo/issues/35)) ([8c541e1](https://github.com/Love-Rox/labolabo/commit/8c541e1b44ce7e87363c7d804b5c2c6bf12fd1e8))

## [0.2.0](https://github.com/Love-Rox/labolabo/compare/v0.1.0...v0.2.0) (2026-07-01)


### 新機能

* Changelog ビューア/リリースノート＋変更ファイルツリー＋サイドバー整形 ([#8](https://github.com/Love-Rox/labolabo/issues/8)) ([40a72ad](https://github.com/Love-Rox/labolabo/commit/40a72ad0423ed3c150024e740ed1f0dc8cab13bc))
* git status(porcelain v2)/diff パーサと単体テストを追加 ([c8eac03](https://github.com/Love-Rox/labolabo/commit/c8eac037016872f24a92bb5b3da55e6e271a1809))
* GitEngine（git status/diff/numstat/worktree 操作）と統合テストを追加 ([4c9a9fa](https://github.com/Love-Rox/labolabo/commit/4c9a9fa64a3d33e488d3738b3faffe2cad5c1171))
* libghostty 端末を埋め込み（タブで複数起動） ([6cd6758](https://github.com/Love-Rox/labolabo/commit/6cd6758e356cb06cefb47d73805ecefae4cda158))
* macOS アプリ殻（XcodeGen + SwiftUI 3ペイン）を追加 ([fd49c88](https://github.com/Love-Rox/labolabo/commit/fd49c889a15e5eb367a5f6ed6aa23ff7daf0b94a))
* New Session 作成フロー（worktree add＋ブランチ＋ベースref） ([#21](https://github.com/Love-Rox/labolabo/issues/21)) ([0f828fd](https://github.com/Love-Rox/labolabo/commit/0f828fd89058a14f2a2089b12387860bc29c0850)), closes [#11](https://github.com/Love-Rox/labolabo/issues/11)
* org ディレクトリの複数リポジトリを横断して扱う WorkPane ([#30](https://github.com/Love-Rox/labolabo/issues/30)) ([fe9ccd6](https://github.com/Love-Rox/labolabo/commit/fe9ccd6539070e22a72f0375de776acd64e43c9a))
* org ディレクトリ配下のリポジトリを個別セッションで一括オープン ([#31](https://github.com/Love-Rox/labolabo/issues/31)) ([e37d508](https://github.com/Love-Rox/labolabo/commit/e37d508606f74e6c0ed11ff3745c91b8dc079879))
* WorkPane のライブ Git 差分（変更ファイル一覧・Diff⇄全文・FSEvents 自動更新） ([62d60aa](https://github.com/Love-Rox/labolabo/commit/62d60aa32bc88cd33f708a3e0bd5c2e4f1dff684))
* worktree 削除アクション（dirty ガード・確認ダイアログ・git worktree remove） ([#23](https://github.com/Love-Rox/labolabo/issues/23)) ([c446b7b](https://github.com/Love-Rox/labolabo/commit/c446b7b227d16d23e2b942f18844e4bcc698e94a)), closes [#13](https://github.com/Love-Rox/labolabo/issues/13)
* アプリアイコン（Figma ライム稲妻）＋カラーモード切替・設定画面 ([#20](https://github.com/Love-Rox/labolabo/issues/20)) ([6f058d8](https://github.com/Love-Rox/labolabo/commit/6f058d836cecf871c2d777fa74e18a8ef2383b25))
* エージェントセッションを永続化し再起動時に --resume で継続 ([#22](https://github.com/Love-Rox/labolabo/issues/22)) ([4c1060e](https://github.com/Love-Rox/labolabo/commit/4c1060e60b08d7caa2cb46261c18c52cd950e2cd)), closes [#12](https://github.com/Love-Rox/labolabo/issues/12)
* エージェント入力待ちの可視化＋macOS 通知 ([#26](https://github.com/Love-Rox/labolabo/issues/26)) ([7229160](https://github.com/Love-Rox/labolabo/commit/7229160e2bdb9e8300c578433a4ea2836f551289))
* コミット履歴グラフの固定レーン化・コミット差分・VSCode 風ツリー・省略ツールチップ ([#10](https://github.com/Love-Rox/labolabo/issues/10)) ([8eb67d8](https://github.com/Love-Rox/labolabo/commit/8eb67d88408df9b6cf4a8a3e03521638029ee7c9))
* サイドバーを Supacode 風に刷新（リポジトリ集約・色・PR/状態） ([#9](https://github.com/Love-Rox/labolabo/issues/9)) ([bd1c103](https://github.com/Love-Rox/labolabo/commit/bd1c103cc7ed12f1b95dbb5ce7de22d4c2e12089))
* セッションを GRDB 永続化し再起動時に復元 ([#5](https://github.com/Love-Rox/labolabo/issues/5)) ([e3dd4ac](https://github.com/Love-Rox/labolabo/commit/e3dd4acde1a0ab04ddc84361c1ff5f21588a327f))
* セッションを閉じるボタンをサイドバー行のホバー×に移設（右上×を撤去） ([#28](https://github.com/Love-Rox/labolabo/issues/28)) ([2ea3659](https://github.com/Love-Rox/labolabo/commit/2ea36590cf199117f943089a45e310cc253edc16))
* ペイン配置の永続化（セッション別）＋名前付きプリセット ([#25](https://github.com/Love-Rox/labolabo/issues/25)) ([a22d79d](https://github.com/Love-Rox/labolabo/commit/a22d79d3c28336e42a48c3f5f4263f6afd1737a8))
* 中央ペインを AppKit タイル化＋コミットグラフ＋hooks ベース Agent 状態＋上部 1 本バー ([#7](https://github.com/Love-Rox/labolabo/issues/7)) ([1550510](https://github.com/Love-Rox/labolabo/commit/15505108e540f736f07ed520c5dfaf2e670ccc3e))


### バグ修正

* コミットグラフのグラフ列が狭い履歴でも幅を取りすぎる問題を修正 ([#24](https://github.com/Love-Rox/labolabo/issues/24)) ([b9501da](https://github.com/Love-Rox/labolabo/commit/b9501da19740847070432bbe01f03173b9843110))
* サイドバー先頭見出し・状態ドットのパルス・描画チラつきの調整 ([#27](https://github.com/Love-Rox/labolabo/issues/27)) ([105f937](https://github.com/Love-Rox/labolabo/commit/105f937577c57cb45cc9aed1c20feb911b932ebe))
* セッション切替フリッカー解消＋サイドバー件数の視認性改善 ([#29](https://github.com/Love-Rox/labolabo/issues/29)) ([83d0aaf](https://github.com/Love-Rox/labolabo/commit/83d0aaf0eceaf95f309de09472cc5a5f0b262828))
* 差分/全文ビューを上詰め表示に（縦中央寄せを解消） ([76ee718](https://github.com/Love-Rox/labolabo/commit/76ee718f465cfa527bcaec7e0bb153779f8253df))
* 差分ビューに行番号ガターを追加し全幅表示に（見やすさ改善） ([827d307](https://github.com/Love-Rox/labolabo/commit/827d307d7435c4c5d893e27ab9cf8e77f6ec1347))


### ドキュメント

* リポジトリ用 CLAUDE.md を追加（構成・ビルド・ブランチ/版運用） ([#6](https://github.com/Love-Rox/labolabo/issues/6)) ([2803834](https://github.com/Love-Rox/labolabo/commit/2803834a495d43d34b9f9ea9de4d91f50677f25b))

## Changelog

All notable changes are recorded here. This file is maintained automatically by
[release-please](https://github.com/googleapis/release-please) from
[Conventional Commits](https://www.conventionalcommits.org/).
