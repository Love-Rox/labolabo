# Changelog

## [0.8.0](https://github.com/Love-Rox/labolabo/compare/v0.7.0...v0.8.0) (2026-07-16)


### 新機能

* LABOLABO_RS_DATA_DIR でデータディレクトリを上書き可能にする ([f34bf7a](https://github.com/Love-Rox/labolabo/commit/f34bf7aed11f98da9c006541ea65f70355445879))
* labolabo-app の Linux 対応（第7波 a: CI ジョブ・フォント解決・tar.gz パッケージング） ([c76dbce](https://github.com/Love-Rox/labolabo/commit/c76dbce4fdbd943571b0a1bb4ded92b279c784b4))
* labolabo-core にセッション永続化（SQLite ストア）を移植（第4波 c） ([6a42c43](https://github.com/Love-Rox/labolabo/commit/6a42c439f7e71c1e74ac34d66141b209a8a66d4c))
* labolabo-core にタイル/タブ木モデルを移植（保存形式バイト互換 + Swift テスト完全移植） ([6040259](https://github.com/Love-Rox/labolabo/commit/6040259d8855b719cb9d99f83f674787b69fc19b))
* labolabo-core（Rust）に commit graph・worktree・agent 状態/使用量パーサを移植（第2波） ([8d709f6](https://github.com/Love-Rox/labolabo/commit/8d709f6a15e60a005af0e647df79c15013d3a230))
* labolabo-core（Rust）に hooks バス + フォワーダを移植（第4波b） ([0dda544](https://github.com/Love-Rox/labolabo/commit/0dda544d4bc53204f2e1c8e0f640d84211a110e9))
* labolabo-core（Rust）に porcelain v2 / unified diff パーサを移植し Swift 版との出力照合を追加 ([6d77dce](https://github.com/Love-Rox/labolabo/commit/6d77dced570fa9d99306eb657b638c7ebf7dfa16))
* labolabo-core（Rust）にプロセス実行系 + git 実行を移植（第4波a） ([580a403](https://github.com/Love-Rox/labolabo/commit/580a403ad2ebaa5d7586bc710cd8a75316f88ce0))
* **rust-app:** Git 表示をタイルペインとして開けるように ([9498181](https://github.com/Love-Rox/labolabo/commit/949818170981ef6969a577713991949b3a861d64))
* **rust-app:** IME 入力対応とクリップボード貼り付け ([1bc359d](https://github.com/Love-Rox/labolabo/commit/1bc359dab3c9b69e8e32bbb820140ee19b53cd71))
* **rust-app:** labolabo-app の Windows 対応（第7波 c） ([9f38706](https://github.com/Love-Rox/labolabo/commit/9f38706082c5b20c42c443876ce35a45bf52c845))
* **rust-app:** Swift版インポートを起動時の確認ダイアログ方式に変更(第8波d) ([bbb9c62](https://github.com/Love-Rox/labolabo/commit/bbb9c62a872bb2730dca167bf7a4a1b1e0a65785))
* **rust-app:** Swift版セッションをRust版Taskへ読み取り専用インポート ([fa69ea4](https://github.com/Love-Rox/labolabo/commit/fa69ea441323f7c6c846c4e8ce5ebc0c41530c5c))
* **rust-app:** UI クロームを ja/en の 2 言語対応に（rust-i18n + OS ロケール自動選択） ([04854fd](https://github.com/Love-Rox/labolabo/commit/04854fd9962d1462303431b0cda426a4e5c5b2e2))
* **rust-app:** UIクロームをモダン化(第8波a) ([984abe3](https://github.com/Love-Rox/labolabo/commit/984abe3478b845b5e6b82c18c548cb6beffd611f))
* **rust-app:** アプリ内表記を LaboLabo に改名 ([20d4972](https://github.com/Love-Rox/labolabo/commit/20d4972d2cbe7375873297d5b071f7f7d38a9599))
* **rust-app:** サイドバーのパーソナライズ(第10波) — タスクの名前変更・色設定、選択タスクのブランド強調、パス省略表示、端末タブの色付け ([7adad5f](https://github.com/Love-Rox/labolabo/commit/7adad5fc33cafd8586a743138ce9a13256477155))
* **rust-app:** ペイン/タブ・サイドバー・OSファイルのドラッグ&ドロップを実装 ([2949390](https://github.com/Love-Rox/labolabo/commit/2949390c97034df8cfcef2aef07efc7cd53c3130))
* **rust-app:** メニューバー・タスクのアーカイブ/削除・ウィンドウ位置記憶・IDE で開く ([c3f8b9c](https://github.com/Love-Rox/labolabo/commit/c3f8b9c9639128b5169b915de255931c0f7f65f1))
* **rust-app:** 端末タブに OSC 0/2 のライブタイトルを反映（第11波） ([26b340e](https://github.com/Love-Rox/labolabo/commit/26b340e5a070894c214bed7848f0f4e57ebe8502))
* **rust-app:** 見つからないワークツリーをサイドバーで検出・整理できるようにする ([aceefe5](https://github.com/Love-Rox/labolabo/commit/aceefe528cb690edcfac8e3057f53811a11e7451))
* **rust-app:** 起動時アップデート確認とサイドバーバナーを追加 ([5e5569d](https://github.com/Love-Rox/labolabo/commit/5e5569db911d107ee4c34c073fcd80209e5bc917))
* **rust-core:** appState に windowBounds キーの get/set を追加 ([e60d6c9](https://github.com/Love-Rox/labolabo/commit/e60d6c907c2a0988961947549f4b02698a0c743e))
* **rust-core:** hooks/control の Windows Named Pipe トランスポートを実装 ([c3a074d](https://github.com/Love-Rox/labolabo/commit/c3a074d66c0a9ef474535bfd58f455b2cf191360))
* **rust-core:** ToolLocator の Windows 実装と process タイムアウト kill の Windows 対応 ([f44907a](https://github.com/Love-Rox/labolabo/commit/f44907ab8178eb5b270d410a9f6fcf0c34181055))
* **rust-core:** データディレクトリを LaboLabo へ統一し旧 LaboLabo-rs から自動移行 ([37cdc4e](https://github.com/Love-Rox/labolabo/commit/37cdc4e019fad7fcc61c0561cf44c0bbea7e37d1))
* **rust:** Git ペイン（変更ファイル一覧・差分・ライブ更新）を実装 ([d23d0da](https://github.com/Love-Rox/labolabo/commit/d23d0da41502e1fe97b68a4385db4d12fc723c3d))
* **rust:** labolabo-app に Ghostty の色設定（background/foreground/cursor-color/palette/theme）を反映 ([3b3389d](https://github.com/Love-Rox/labolabo/commit/3b3389d149f36974d92d34adf0b7c2408e6d4fb2))
* **rust:** labolabo-app のタブモデルを labolabo-core::tiling のタイル木に置換（第5波b-2） ([42f4b8c](https://github.com/Love-Rox/labolabo/commit/42f4b8c846f2d81f7837216eea2beaef7af2413e))
* **rust:** labolabo-app（gpui 端末シェル）を新設（第5波a） ([89480ec](https://github.com/Love-Rox/labolabo/commit/89480ecdd600cada3fe29f12e153153f44997a68))
* **rust:** labolabo-term クレートを新設（PTY 端末セッションコア） ([3ea0c8c](https://github.com/Love-Rox/labolabo/commit/3ea0c8ce2c0c97bcb5b0c6ab8ba0416a8768d298))
* **rust:** macOS .app バンドル化スクリプトと手動 CI ジョブを追加 ([a1ae389](https://github.com/Love-Rox/labolabo/commit/a1ae3891b739937b4d535b7bbf268876393c5391))
* **rust:** RC リリース配管とバージョン単一ソース化 ([cc5a1b6](https://github.com/Love-Rox/labolabo/commit/cc5a1b6e3533c2390b28c843e8e0c2afd6c9c45e))
* **rust:** UI デザイントークン導入とクローム整備 ([c7ca294](https://github.com/Love-Rox/labolabo/commit/c7ca2940131b6952769d39abe4ddff9c7914298e))
* **rust:** エージェント使用量表示・セッション間競合バッジ・設定画面を追加 ([8a0e52c](https://github.com/Love-Rox/labolabo/commit/8a0e52c7b7be6f85c6bdb9460b15be8fef8b7a88))
* **rust:** コントロール CLI と control-protocol の仕様/実装を追加 ([e2fcb87](https://github.com/Love-Rox/labolabo/commit/e2fcb8717e4f0c1346ae8cbcdba9b94848047643))
* **rust:** タスクモデル実装 — サイドバー + 1 作業 = 1 タイル木 + SQLite 永続化 (plans/012 §1) ([6707ebd](https://github.com/Love-Rox/labolabo/commit/6707ebd88249349171c55049c754d92515fb548e))
* Rust版 labolabo-app に Claude Code hooks 統合を実装 ([d7d2a62](https://github.com/Love-Rox/labolabo/commit/d7d2a62e01a53701ce8edeecd06579f0fd4aeb5d))
* **rust:** 端末のスクロールバック表示・テキスト選択・Cmd+C コピーを実装 ([cd015b1](https://github.com/Love-Rox/labolabo/commit/cd015b1ea834d72860bf7380d42db0782189ee40))
* **rust:** 端末のマウスレポーティング・仕切りドラッグリサイズ・単語/行選択を追加 ([88b601c](https://github.com/Love-Rox/labolabo/commit/88b601cd15cd1ca2a373a9f8335f9cde0e1c9fc3))


### バグ修正

* Linux ビルドで dead code になる macOS 専用 UI ヘルパーの clippy エラーを解消 ([d249e55](https://github.com/Love-Rox/labolabo/commit/d249e55d67af6ab7be5123844240f699630c9003))
* Linux ビルドで未使用になる import を cfg でゲート（clippy -D warnings 対応） ([3494c9d](https://github.com/Love-Rox/labolabo/commit/3494c9dcb29b489184772fb0ac87c8bad4bcec77))
* **rust-core:** swift_import のテストを実git依存分だけ #[cfg(unix)] に ([b60381a](https://github.com/Love-Rox/labolabo/commit/b60381a835111075a9d6b904e8a495e28636de1f))
* **rust:** attached 作業のディレクトリはピッカーで選んだパスをそのまま使う ([ba57f01](https://github.com/Love-Rox/labolabo/commit/ba57f0165b80bf90ad162ea57bf06bca84825826))
* **rust:** FileWatcher テストの Windows パス区切り依存を除去 ([0a89737](https://github.com/Love-Rox/labolabo/commit/0a8973795659a5a3397061d339da72e52530c816))
* **rust:** labolabo-app の実機フィードバック2件を修正（exit 終了処理・Ghostty フォント設定） ([d8bf0de](https://github.com/Love-Rox/labolabo/commit/d8bf0debeb726b2d76c724a9bef7c98f51b8cadf))
* **rust:** labolabo-hook が --hook フラグ形式の呼び出しを受け付けるようにする ([2543723](https://github.com/Love-Rox/labolabo/commit/25437235c4b0a1ea05843affe1b0397ab2ce3587))
* **rust:** scrollback キャップの回帰テストを backend-ghostty-vt の実挙動に合わせる ([e0b89c8](https://github.com/Love-Rox/labolabo/commit/e0b89c858a296eb58e910e9b5bd3352c4316a1f0))
* **rust:** 端末セルの寸法をデバイスピクセル四捨五入にする（字間が空く問題の修正） ([c3d10ee](https://github.com/Love-Rox/labolabo/commit/c3d10ee82e5cde8df094df4f3e016be895843c9d))
* **rust:** 選択中の全角文字の右半分欠けと IME 下線の分断を修正 ([7ee677e](https://github.com/Love-Rox/labolabo/commit/7ee677e5341e5d2849949fe0606780ba009a6e1e))


### リファクタリング

* Darwin 依存を条件コンパイルで分離し Linux ビルドを可能にする ([15bec9a](https://github.com/Love-Rox/labolabo/commit/15bec9a6da0018d0cc798b45afdce2159006036e))
* hooks のトランスポートと解釈を分離し、ワイヤプロトコル仕様書を追加 ([ed63f0f](https://github.com/Love-Rox/labolabo/commit/ed63f0f3094c1059aaf30423d05c88cacd1cd828))
* エンジンの OS 依存面（プロセス実行・ツール解決・ファイル監視）を protocol 化 ([6124c04](https://github.com/Love-Rox/labolabo/commit/6124c04dd3bd9ff1f5c1e0812588bce9cdf3dabf))
* タイル/タブ木モデルを UI 非依存ファイルへ分離（AppKit 依存を除去） ([d57f7d8](https://github.com/Love-Rox/labolabo/commit/d57f7d825c12ae91c6790556c4970ae57f320181))
* 永続化を SessionPersisting protocol の背後へ隔離し、データディレクトリ解決を集約 ([b520580](https://github.com/Love-Rox/labolabo/commit/b52058043f0313fae4e95676861260215bb0480f))


### ドキュメント

* **rust:** README に Windows コア波（Named Pipe / ToolLocator / taskkill）の節を追記 ([7777845](https://github.com/Love-Rox/labolabo/commit/77778456aa0f4942b7b706e43ddae04c3a3eee46))
* **rust:** README を LaboLabo 改名に追随 ([b10b4a3](https://github.com/Love-Rox/labolabo/commit/b10b4a3048d65bdc8454fe3192f2d90a7960433d))

## [0.7.0](https://github.com/Love-Rox/labolabo/compare/v0.6.2...v0.7.0) (2026-07-12)


### 新機能

* Claude セッションのタブ別記憶と自動 resume・終了時保存 ([753bd70](https://github.com/Love-Rox/labolabo/commit/753bd707c87e544c0b83ed6fb1e2fe97ce699bf2))
* UI アニメーションを整備（モーショントークン・電力/Reduce Motion 対応） ([cda417b](https://github.com/Love-Rox/labolabo/commit/cda417b6749da62d7e089a8345db34e355f24e52))
* 端末ペインのタブ化とフォーカス制御 ([749cc10](https://github.com/Love-Rox/labolabo/commit/749cc10f31d5b820b32bd3c8de68fe3092b53163))

## [0.6.2](https://github.com/Love-Rox/labolabo/compare/v0.6.1...v0.6.2) (2026-07-10)


### バグ修正

* ツリー表示でファイル行をクリックしても選択されず diff が出ない問題を修正 ([#83](https://github.com/Love-Rox/labolabo/issues/83)) ([1780674](https://github.com/Love-Rox/labolabo/commit/1780674b88c708cc986052a30ada12747c2b7c93))

## [0.6.1](https://github.com/Love-Rox/labolabo/compare/v0.6.0...v0.6.1) (2026-07-08)


### バグ修正

* git 実行のスレッド枯渇デッドロックを根治し終了ハング・表示凍結を解消 ([#78](https://github.com/Love-Rox/labolabo/issues/78)) ([4df0085](https://github.com/Love-Rox/labolabo/commit/4df0085300f120c3f84b03bbd0901fa4abdc98cc))


### パフォーマンス

* 使い捨て SessionStore init の副作用排除と refresh デバウンスで常時 CPU を削減 ([#80](https://github.com/Love-Rox/labolabo/issues/80)) ([a1da3b6](https://github.com/Love-Rox/labolabo/commit/a1da3b6116c703857c5d78120bb7b65d86ff2e59))

## [0.6.0](https://github.com/Love-Rox/labolabo/compare/v0.5.1...v0.6.0) (2026-07-07)


### 新機能

* 外観のライト/ダーク/システム準拠を設定から選択可能に ([#75](https://github.com/Love-Rox/labolabo/issues/75)) ([adda3c9](https://github.com/Love-Rox/labolabo/commit/adda3c9a5ee9e7a18a4e8486c787dd8de4d1f7c6))

## [0.5.1](https://github.com/Love-Rox/labolabo/compare/v0.5.0...v0.5.1) (2026-07-06)


### バグ修正

* ＋メニューの「既存のフォルダを開く…」でフォルダ選択パネルが開かない問題を修正 ([#72](https://github.com/Love-Rox/labolabo/issues/72)) ([3347b16](https://github.com/Love-Rox/labolabo/commit/3347b16f97deb22f35f66358965e812015921434))

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
