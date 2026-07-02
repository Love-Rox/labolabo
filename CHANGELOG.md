# Changelog

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
