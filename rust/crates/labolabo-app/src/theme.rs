//! UI クロームのデザイントークン (`plans/013-rust-ui-design-tokens.md`).
//!
//! LaboLabo の UI クロームの仕事は「端末を主役に保ちつつ、いま誰が自分を
//! 必要としているかを一瞥で読めるようにする」こと。パレットは純グレーでは
//! なく、わずかに青みのある寒色ダークニュートラル(計器盤の趣) -- ユーザー
//! の端末色が画面上で最も「温かい」ものになるよう、あえて彩度を落として
//! ある。
//!
//! **スコープ外**: 端末セル内の色(ユーザーの Ghostty 設定が正)。
//! `crate::render::paint_grid`/`paint_cursor`/`paint_preedit` のセル背景・
//! 文字色・カーソル色はここの対象ではない -- 唯一の例外は選択ハイライト
//! (`render::SELECTION_HIGHLIGHT_RGB`)で、これはフォーカス枠と同じ
//! [`ACCENT`] を使うことで「このペインにフォーカスがある」と「これが選択
//! 範囲だ」を同じ視覚的ファミリーとして読ませる、意図した例外。

/// 背景面。奥まった(SUNKEN)ものから持ち上がった(ACTIVE)ものへ、4 段階。
pub mod surface {
    /// サイドバー・Git ペインなど、固定サイドパネルの最も奥まった面。
    pub const SUNKEN: u32 = 0x101114;
    /// ウィンドウ基調・設定オーバーレイの地。
    pub const ROOT: u32 = 0x141518;
    /// タブバー・パネルヘッダなど、SUNKEN よりわずかに持ち上がった面。
    pub const RAISED: u32 = 0x1d1f24;
    /// 選択チップ・選択行・ボタンなど、操作対象として強調された面。
    pub const ACTIVE: u32 = 0x2a2d34;
    /// ヘアライン境界線。
    pub const STROKE: u32 = 0x2c2f36;
}

/// 本文テキストの明度 3 段階。
pub mod text {
    pub const PRIMARY: u32 = 0xe8eaed;
    pub const SECONDARY: u32 = 0x9aa0a8;
    pub const MUTED: u32 = 0x6b7077;
    /// [`super::ACCENT`] 地(選択中のトグルピル等)に載せる文字色。他の
    /// 3 段階と違い明るい面の上で使うので、単独で暗く固定している。
    pub const ON_ACCENT: u32 = 0x0a0e14;
}

/// フォーカス・選択の確立済みアクセント(既存値を維持)。
pub const ACCENT: u32 = 0x5e9eff;

/// エージェント状態ドット(`task_workspace::status_dot_color`)の色。
pub mod status {
    pub const STARTING: u32 = 0xff9f0a;
    pub const RUNNING: u32 = 0x30d158;
    pub const WAITING: u32 = 0xffd60a;
    pub const IDLE: u32 = 0x8e8e93;
    pub const ENDED: u32 = 0x555a60;
    /// 警告・競合バッジ・エラー文言 -- サイドバーの ⚠ と new-task エラーで
    /// 共有(`plans/013` 手順 2 の "sidebar.rs の ⚠ 0xffa500・0xff6b6b →
    /// status::CONFLICT に統一" 通り、従来 2 色だったものを 1 色へ)。
    pub const CONFLICT: u32 = 0xff6b6b;
}

/// Git 差分の追加/削除行。
pub mod diff {
    pub const ADD: u32 = 0x3fb950;
    pub const DEL: u32 = 0xf85149;
    /// 追加/削除行の淡い背景色 -- `plans/013` の対応表には無いが、"51 箇所
    /// 全置換 / theme 経由以外の rgb(0x..) はゼロに" という手順 2 の原則に
    /// 従い、`git_pane.rs` の `ADDITION_BG`/`DELETION_BG` をここへ追加した
    /// トークン(逸脱ではなく、原則を徹底するための拡張)。
    pub const ADD_BG: u32 = 0x14251a;
    pub const DEL_BG: u32 = 0x2a1616;
}

/// ドラッグ&ドロップのハイライト。色相は既存のものを維持しつつ一箇所に
/// 集約(`sidebar.rs`/`task_workspace.rs` に散っていた `0x30d158.."`
/// 系リテラルの重複定義を解消)。
pub mod dnd {
    /// サイドバーの Task 行並び替えドロップ時のハイライト(緑)。
    /// `status::RUNNING` と同じ色相だが意味は無関係 -- 偶然の一致であり、
    /// 「実行中」の意味を借りているわけではないので独立したトークンに
    /// している。
    pub const REORDER: u32 = 0x30d158;
    /// タイル/タブの移動ドロップ時のハイライト(青、[`ACCENT`] と同色 --
    /// フォーカス枠と同じ「これは操作対象」というファミリー)。
    pub const MOVE: u32 = super::ACCENT;
    /// OS ファイル/フォルダの挿入ドロップ時のハイライト(琥珀 --
    /// `status::STARTING` とは意味が異なる独立の用途なので分けている)。
    pub const FILE_INSERT: u32 = 0xff9f0a;
}

/// タブチップ/サイドバー行などラベル文字の等幅化(`plans/013` 手順 3)。
pub mod font_size {
    /// タブチップ・サイドバー行のタイトル。
    pub const LABEL: f32 = 12.0;
    /// 使用量ラベル・差分 +/- 数・リポジトリグループ見出しなどの補助文字。
    pub const CAPTION: f32 = 11.0;
}

/// `rgb` トークン(`0xRRGGBB`)に 8bit アルファを合成し、`gpui::rgba` へ渡せる
/// `0xRRGGBBAA` を返す。ドロップハイライト/ホバー色など、同じ色相に複数の
/// 透明度が必要な箇所で色相の重複リテラルを避けるためのヘルパー。
pub const fn with_alpha(rgb: u32, alpha: u8) -> u32 {
    (rgb << 8) | alpha as u32
}

/// パネル/オーバーレイの角丸半径 (`plans` 第8波a §1/§3/§5: 「クロームの
/// 見た目を一段引き上げる」磨きの波)。`f32`(px 値)で持ち、呼び出し側が
/// `.rounded(px(RADIUS))`/`.rounded_r(px(RADIUS))` 等へそのまま渡す --
/// gpui 0.2 のプリセット(`rounded_lg`=8px 等)と値としては一致する場合も
/// あるが、「このアプリの意味付けとしてのパネル角丸」を一箇所で管理する
/// ため、プリセット名への暗黙の依存はしない。
pub mod radius {
    /// 固定パネル(サイドバー/Git ペイン)の、ワークスペースに面した側だけ
    /// 丸める角丸。ウィンドウ端に接する側は直角のまま -- 「ウィンドウ
    /// 全体は端まで surface::ROOT、パネルはその上に載るカードに」の実装。
    pub const PANEL: f32 = 8.0;
    /// サイドバーのタスク行/アーカイブ行の角丸。
    pub const ROW: f32 = 6.0;
    /// オーバーレイ(設定・確認・「…」メニュー・About)パネルの角丸 --
    /// `PANEL` よりわずかに大きい(画面全体の上に浮くモーダルなので、
    /// 固定パネルより一段強い丸みで「最前面」を印象づける)。
    pub const OVERLAY: f32 = 10.0;
}

/// オーバーレイ(設定・確認・「…」メニュー・About)背景の暗幕、黒 40%。
/// 全オーバーレイで統一 -- 従来は設定/確認/About がそれぞれ個別に
/// `0xb3`(≈70%)を定義し、タスク「…」メニューのポップオーバーは暗幕
/// 無し(クリックアウトで閉じるだけ)と、オーバーレイごとにバラバラ
/// だったものをここへ集約した。
pub const OVERLAY_SCRIM: u32 = with_alpha(0x000000, 0x66);

/// ごく控えめな box-shadow のプリセット(`plans` 第8波a §1/§4: 「浮き」の
/// 階層とフォーカスの外側グロー)。電力原則により、繰り返しアニメーション
/// には一切乗せない(常に静的な style として描画されるだけ -- `motion.rs`
/// の「単発 ≤250ms か、Running の呼吸だけ」原則の対象外: そもそも
/// アニメーションしない)。ぼかし半径も控えめに抑えている(重い blur は
/// 見た目の主張が強すぎてこのアプリの「端末が主役」トーンと合わない、
/// という意味も込め)。
pub mod shadow {
    use gpui::{point, px, rgba, BoxShadow};

    /// 固定パネル(サイドバー/Git ペイン/各タイルのタブバー)の「浮き」を
    /// 示す、1 段だけのシャドウ。`offset_x`/`offset_y` は呼び出し側が
    /// パネルがどちら向きに「浮いて」見えるべきかを渡す(サイドバーなら
    /// 右へ、Git ペインなら左へ、タブバーなら下へ)。
    pub fn panel(offset_x: f32, offset_y: f32) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: rgba(super::with_alpha(0x000000, 0x38)).into(),
            offset: point(px(offset_x), px(offset_y)),
            blur_radius: px(8.0),
            spread_radius: px(0.0),
        }]
    }

    /// オーバーレイ(設定・確認・「…」メニュー・About)パネル用 -- 画面
    /// 全体の上に浮くモーダルなので [`panel`] よりわずかに強い。
    pub fn overlay() -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: rgba(super::with_alpha(0x000000, 0x40)).into(),
            offset: point(px(0.0), px(4.0)),
            blur_radius: px(14.0),
            spread_radius: px(0.0),
        }]
    }

    /// フォーカスされたタイルの外側グロー(`plans` 第8波a §4: 「1px
    /// ACCENT の枠 + ほんのり ACCENT の外側グロー 1 段」)。オフセット
    /// 無し・低アルファの [`super::ACCENT`] を小さく広げるだけの、枠線を
    /// 縁取る程度のごく控えめなグロー。
    pub fn focus_glow() -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: rgba(super::with_alpha(super::ACCENT, 0x40)).into(),
            offset: point(px(0.0), px(0.0)),
            blur_radius: px(6.0),
            spread_radius: px(1.0),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_alpha_shifts_rgb_and_appends_the_alpha_byte() {
        assert_eq!(with_alpha(0x5e9eff, 0x4d), 0x5e9eff4d);
        assert_eq!(with_alpha(0x000000, 0xff), 0x000000ff);
        assert_eq!(with_alpha(ACCENT, 0x00), 0x5e9eff00);
    }
}
