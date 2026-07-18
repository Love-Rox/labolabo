//! LaboLabo Rust 版のモーションシステム (`plans/014-rust-motion-system.md`)。
//!
//! ## 原則 (このアプリ固有・厳守)
//!
//! 1. **電力最優先**: 繰り返しアニメーションは「Running 状態のドット」の
//!    呼吸(M2)*のみ*。他はすべて `≤250ms` の単発。アニメーション中だけ
//!    再描画が走り、終われば完全に止まる -- 実際にこれを担保しているのは
//!    gpui 自身の [`gpui::AnimationElement`] の挙動で、`oneshot` な
//!    [`gpui::Animation`] は経過時間が `duration` を超えた時点で
//!    `window.request_animation_frame()` を呼ばなくなる(gpui 0.2 の
//!    `elements/animation.rs` 参照)。このモジュールの各ヘルパーはその上に
//!    薄く乗るだけで、独自のタイマー/ポーリングは一切持たない。
//! 2. イージング: 出現 = [`ease_out_strong`]、色変化 = [`ease_standard`]。
//!    `ease-in` は使用禁止。
//! 3. **Reduce Motion**: [`reduce_motion`] が `true` を返す間、呼吸(M2)は
//!    静的表示に倒す(位置移動系のアニメーションのみ削除、単発の色/不透明度
//!    遷移は残してよい、という原則どおり呼び出し側で使い分ける)。

use std::cell::Cell;
use std::time::Duration;

use gpui::{div, prelude::*, px, rgb, rgba, Animation, AnimationExt, AnyElement, SharedString};

/// 状態ドットの色クロスフェード (M1) にかける時間。
pub const DOT_CROSSFADE: Duration = Duration::from_millis(200);
/// Running の呼吸 (M2) の 1 周期。
pub const BREATH_PERIOD: Duration = Duration::from_millis(1200);
/// DnD ドロップゾーンハイライトの出現 (M3)。
pub const DROP_ZONE_FADE_IN: Duration = Duration::from_millis(120);
/// 設定オーバーレイの入場 (M4)。
pub const OVERLAY_ENTER: Duration = Duration::from_millis(180);
/// 設定オーバーレイの退場 (M4) -- 今回は即時クローズを選んだため未使用だが、
/// 「実装コストが高ければ即時クローズで可」という判断を PR に残すための
/// 記録として定数だけ残してある。
#[allow(dead_code)]
pub const OVERLAY_EXIT: Duration = Duration::from_millis(120);

/// 呼吸/ドット径周りの寸法トークン。値そのものは `plans/013` のスコープだが
/// (色ではなくサイズなので theme.rs には置いていない)、`task_workspace.rs`/
/// `sidebar.rs` の 2 箇所で重複させないためここに集約。
pub const STATUS_DOT_SIZE: f32 = 6.0;

/// 統合ドット (`plans` 第16波 #2) の外輪の太さ。カスタム色を持つタスク/
/// タブだけに描く -- [`unified_dot_element`] のドキュメント参照。
pub const DOT_RING_WIDTH: f32 = 1.5;

/// 統合ドット全体(外輪込み)の一辺 -- [`STATUS_DOT_SIZE`] の中身を
/// [`DOT_RING_WIDTH`] の輪が両側から囲む分だけ大きい。呼び出し側
/// (`task_workspace.rs`/`sidebar.rs`)はカスタム色の有無に関わらずこの
/// サイズで行内の割り付け枠を確保する -- 行によって輪の有無でドットの
/// 占有幅が変わり、隣接要素がガタつくのを防ぐため。
pub const DOT_RING_SIZE: f32 = STATUS_DOT_SIZE + DOT_RING_WIDTH * 2.0;

/// `LABOLABO_REDUCE_MOTION=1` を見て有効化する簡易 Reduce Motion フック。
/// gpui 0.2 には macOS の「視差効果を減らす」設定を直接読む API が無い
/// (`NSWorkspace`/`defaults` を自前で読むのはこの用途には過剰) ので、
/// `plans/014` の指示どおり env var フックのみ実装し、OS 連動は TODO と
/// して残す。
///
/// TODO(OS 連動): macOS では `NSWorkspace.shared.accessibilityDisplayShouldReduceMotion`
/// を購読すれば実現できる -- 将来 gpui 側に相当する公開 API が生えたら
/// それに置き換える。
pub fn reduce_motion() -> bool {
    reduce_motion_from(std::env::var("LABOLABO_REDUCE_MOTION").ok())
}

/// [`reduce_motion`]'s actual decision, pulled out as a pure function of an
/// already-read env value so it's unit-testable without mutating real
/// process-wide env state (which would race against other tests running in
/// the same `cargo test` binary).
fn reduce_motion_from(value: Option<String>) -> bool {
    value.as_deref() == Some("1")
}

// ============================================================================
// イージング -- 純関数、cubic-bezier(x1,y1,x2,y2) の Newton-Raphson 実装
// ============================================================================

/// 出現用の strong ease-out: `cubic-bezier(0.23, 1.0, 0.32, 1.0)`。
pub fn ease_out_strong() -> impl Fn(f32) -> f32 {
    cubic_bezier(0.23, 1.0, 0.32, 1.0)
}

/// 色変化用の ease (CSS 標準の `ease` と同じ曲線):
/// `cubic-bezier(0.25, 0.1, 0.25, 1.0)`。
pub fn ease_standard() -> impl Fn(f32) -> f32 {
    cubic_bezier(0.25, 0.1, 0.25, 1.0)
}

/// `cubic-bezier(x1, y1, x2, y2)` の unit bezier (両端が (0,0)/(1,1) 固定)
/// を、CSS/Web 標準と同じ意味論で `x -> y` の関数として返す。`x` は
/// Newton-Raphson でパラメータ `t` に逆算してから `y(t)` を評価する
/// (`x(t)` は `x1,x2` が通常 `[0,1]` の範囲にある限り単調なので収束する)。
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 {
    move |x: f32| {
        let x = x.clamp(0.0, 1.0);
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        let t = solve_t_for_x(x, x1, x2);
        bezier_component(t, y1, y2)
    }
}

fn bezier_component(t: f32, p1: f32, p2: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
}

fn bezier_derivative(t: f32, p1: f32, p2: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * p1 + 6.0 * u * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

fn solve_t_for_x(x: f32, x1: f32, x2: f32) -> f32 {
    let mut t = x;
    for _ in 0..8 {
        let x_at_t = bezier_component(t, x1, x2) - x;
        let dx = bezier_derivative(t, x1, x2);
        if dx.abs() < 1e-6 {
            break;
        }
        t -= x_at_t / dx;
        t = t.clamp(0.0, 1.0);
    }
    t
}

/// Running ドットの「呼吸」カーブ (M2): 周期上の位置 `t` (`0.0..=1.0`、
/// [`Animation::repeat`] が周期ごとに `0..1` へ巻き戻して渡す) を
/// opacity `1.0 -> 0.55 -> 1.0` の ease-in-out にマップする。gpui 標準の
/// `pulsating_between` と同じ「余弦で自然に減速/加速する」考え方だが、
/// 自前で持つことで純関数としてユニットテストできるようにしてある。
pub fn breath_opacity(t: f32) -> f32 {
    const MIN: f32 = 0.55;
    const MAX: f32 = 1.0;
    let t = t.rem_euclid(1.0);
    // 1.0 (t=0) -> 0.0 (t=0.5) -> 1.0 (t=1) の余弦。両端で速度 0 になる
    // ため、そのまま ease-in-out として機能する。
    let raw = (1.0 + (t * std::f32::consts::TAU).cos()) / 2.0;
    MIN + raw * (MAX - MIN)
}

// ============================================================================
// M1: 状態ドットの色クロスフェード
// ============================================================================

/// 状態ドット 1 個ぶんの「直前に表示していた色」を保持する、`Cell` 経由で
/// `&self` からでも更新できる小さな状態(`plans/014` M1 の「ペインごとに
/// (前回色, 変化時刻) を保持」の実体)。
///
/// `started_at`/経過時間は自前で持たない: 実際のタイミングは
/// [`gpui::AnimationElement`] 自身の内部状態(要素 id ごとに保持される)に
/// 完全に委ねている -- [`advance_dot`] が変化のたびに要素 id を変える
/// (`generation` を進める)ことで、gpui 側に「これは新しいアニメーション
/// だ」と気付かせて自動的にタイマーを再スタートさせる仕組み。これにより
/// このモジュールは実時間を一切ポーリングしない(このモジュール自身は
/// 純粋な状態遷移だけを持つ)。
#[derive(Debug, Clone, Copy, Default)]
pub struct DotAnimState {
    /// 直近の `advance_dot` 呼び出し時点で表示対象だった色。
    shown: Option<u32>,
    /// 直前の遷移における「遷移元」の色。
    from: Option<u32>,
    /// 変化のたびに 1 増える -- アニメーション要素の id に混ぜ込むことで
    /// 「同じ色に見えても別の遷移」を gpui に区別させる。
    generation: u64,
}

/// [`advance_dot`] が返す、いま描画すべき 1 フレームぶんの情報。
#[derive(Debug, Clone, Copy)]
pub struct DotFrame {
    pub from: Option<u32>,
    pub to: Option<u32>,
    pub generation: u64,
}

/// `anim` に記録された前回の表示色と `target` を比較し、変化していれば
/// 遷移(`from`/`generation`)を記録してから、常にこのフレームの
/// `(from, to, generation)` を返す純粋な状態遷移(gpui 型に依存しない)。
/// `target == None` かつまだ一度も色が付いたことがなければ `from`/`to`
/// ともに `None` になる -- 呼び出し側はこの場合ドット自体を描画しない
/// (何も描かない = アニメーション要素も一切生成しない = 追加コスト無し)。
pub fn advance_dot(anim: &Cell<DotAnimState>, target: Option<u32>) -> DotFrame {
    let mut state = anim.get();
    if state.shown != target {
        state = DotAnimState {
            shown: target,
            from: state.shown,
            generation: state.generation.wrapping_add(1),
        };
        anim.set(state);
    }
    DotFrame {
        from: state.from,
        to: state.shown,
        generation: state.generation,
    }
}

/// `from`/`to` (ともに `0xRRGGBB` または「ドット無し」の `None`) の間を
/// 補間した `0xRRGGBBAA` を返す純関数。`None` は「もう一方の色を透明で」
/// と解釈するので、色↔ドット無しの遷移は瞬時のオン/オフではなく自然な
/// フェード in/out になる (`plans/014` M1 の要求どおり)。RGB は各チャンネル
/// の線形補間、アルファも同様に線形補間する。
pub fn lerp_dot_color(from: Option<u32>, to: Option<u32>, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let (from_rgb, from_a) = match from {
        Some(c) => (c, 1.0),
        None => (to.unwrap_or(0), 0.0),
    };
    let (to_rgb, to_a) = match to {
        Some(c) => (c, 1.0),
        None => (from.unwrap_or(0), 0.0),
    };
    let r = lerp_channel(from_rgb, to_rgb, 16, t);
    let g = lerp_channel(from_rgb, to_rgb, 8, t);
    let b = lerp_channel(from_rgb, to_rgb, 0, t);
    let a = (lerp_f32(from_a, to_a, t) * 255.0)
        .round()
        .clamp(0.0, 255.0) as u32;
    (r << 24) | (g << 16) | (b << 8) | a
}

fn lerp_channel(from_rgb: u32, to_rgb: u32, shift: u32, t: f32) -> u32 {
    let from = ((from_rgb >> shift) & 0xff) as f32;
    let to = ((to_rgb >> shift) & 0xff) as f32;
    lerp_f32(from, to, t).round().clamp(0.0, 255.0) as u32
}

fn lerp_f32(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

/// 状態ドット 1 個を組み立てる、`task_workspace.rs`/`sidebar.rs` 共用の
/// ヘルパー。`target`/`is_running` は呼び出し側が
/// `task_workspace::status_dot_color`/`AgentStatus` から求めた値を渡す
/// (このモジュール自身は `AgentStatus` を知らない)。
///
/// - `is_running && breathing_enabled` の間は [`BREATH_PERIOD`] 周期の
///   呼吸(M2)だけを掛ける -- `breathing_enabled` は呼び出し側で「ウィンド
///   ウがアクティブか」と「Reduce Motion か」をすでに畳み込んだ値を渡す
///   (`app::LaboLaboApp::render` 参照)。
/// - それ以外で色が変化した直後は [`DOT_CROSSFADE`] だけ
///   [`ease_standard`] でクロスフェードする(M1)。M1 と M2 は同一要素に
///   同時には掛からない(このフレームごとの排他の理由は関数本体のコメント
///   参照)。
/// - 一度も色が付いたことがなく `target` も `None` なら `None` を返す
///   (要素自体を作らない -- 電力コストゼロ)。
///
/// 一度でも色が付いたドットは、`target` が `None` に戻った(フェード
/// アウトした)後も透明な要素として残り続ける -- `from` を消さない設計
/// にしている代わりに、gpui の oneshot アニメーションは完了後
/// `request_animation_frame` を呼ばなくなるため追加のフレーム要求は
/// 発生しない(このモジュール冒頭の doc 参照)。経過時間を自前で追跡して
/// 完全に要素を取り除く最適化は、電力上のメリットが無い割にコードが
/// 複雑になるため見送っている。
pub fn status_dot_element(
    id_base: impl std::fmt::Display,
    target: Option<u32>,
    is_running: bool,
    breathing_enabled: bool,
    anim: &Cell<DotAnimState>,
) -> Option<AnyElement> {
    let frame = advance_dot(anim, target);
    if frame.from.is_none() && frame.to.is_none() {
        return None;
    }

    let base = div()
        .w(px(STATUS_DOT_SIZE))
        .h(px(STATUS_DOT_SIZE))
        .rounded_full();

    // M1 (color crossfade) and M2 (Running breathing) are deliberately
    // mutually exclusive per frame, not composed: `gpui::AnimationElement`
    // doesn't re-expose `Styled` on its wrapped result (it only implements
    // `Element`/`IntoElement`), so a second `.with_animation()` chained on
    // top of the first has no `.bg()`/`.opacity()` to call inside its own
    // animator -- there is no supported way in gpui 0.2 to run two
    // independently-timed style animations (a 200ms oneshot color fade and
    // a 1200ms repeating opacity pulse) on the same element at once. While
    // breathing owns the dot, its color simply renders at its settled
    // `target` (the common case anyway -- a pane's status rarely changes
    // color *while* it's Running); the crossfade only plays when breathing
    // isn't active for this frame.
    if is_running && breathing_enabled {
        let color = target.unwrap_or(0);
        let breathe_id = SharedString::from(format!("{id_base}-breathe"));
        Some(
            base.bg(rgb(color))
                .with_animation(
                    breathe_id,
                    Animation::new(BREATH_PERIOD)
                        .repeat()
                        .with_easing(breath_opacity),
                    |el, opacity| el.opacity(opacity),
                )
                .into_any_element(),
        )
    } else {
        let from = frame.from;
        let to = frame.to;
        let crossfade_id = SharedString::from(format!("{id_base}-fade-{}", frame.generation));
        Some(
            base.with_animation(
                crossfade_id,
                Animation::new(DOT_CROSSFADE).with_easing(ease_standard()),
                move |el, t| el.bg(rgba(lerp_dot_color(from, to, t))),
            )
            .into_any_element(),
        )
    }
}

/// 統合ドット (`plans` 第16波 #2) の描画パラメータ -- 「状態ドット + カスタム
/// 色ドット」の 2 個表示を 1 個に統合する設計の核: **外輪 = カスタム色、
/// 中の塗り = 状態色**という写像を、gpui 抜きの純関数として
/// [`unified_dot_params`] に切り出してある(ユニットテストの対象はこちら --
/// 実際の要素組み立ては [`unified_dot_element`] 参照)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnifiedDotParams {
    /// `Some(color)` なら [`DOT_RING_WIDTH`] 幅の輪をこの色で描く(タスク/
    /// タブのカスタム色)。`None` は「輪なし」-- 色未設定のタスク/タブは
    /// 第16波以前と見た目が変わらない(現状ドットのまま)。
    pub ring_color: Option<u32>,
    /// 中の塗り(状態ドット)へそのまま渡す `target` -- クロスフェード/
    /// 呼吸を含む挙動は [`status_dot_element`] 側の既存ロジックを完全に
    /// 再利用する(この構造体はどの色を塗るかを渡すだけ)。
    pub fill_target: Option<u32>,
}

/// (状態色, カスタム色) -> [`UnifiedDotParams`]。外輪は常にカスタム色を
/// そのまま、中の塗りは常に状態色をそのまま渡すだけの写像だが、「統合」の
/// 契約(輪と塗りが互いに独立で、どちらの入力がどちらの見た目に効くか)を
/// 一箇所に固定し、[`unified_dot_element`] からもテストからも同じ答えを
/// 参照できるようにするために関数として切り出している。
pub fn unified_dot_params(
    status_color: Option<u32>,
    custom_color: Option<u32>,
) -> UnifiedDotParams {
    UnifiedDotParams {
        ring_color: custom_color,
        fill_target: status_color,
    }
}

/// [`status_dot_element`] を包み、カスタム色があれば [`DOT_RING_WIDTH`]
/// 幅の輪を追加で描く、`task_workspace.rs`/`sidebar.rs` 共用のヘルパー
/// (`plans` 第16波 #2)。
///
/// - カスタム色が無ければ [`status_dot_element`]/[`static_dot_fill`] の
///   結果をそのまま返す(輪の分のラッパーすら作らない -- 色未設定タスクは
///   第16波以前と完全に同じ描画・同じコスト)。
/// - カスタム色があれば、中の塗りが `None`(= まだ一度も状態が付いたことが
///   ない)であっても輪だけは必ず描く -- 「状態 None かつ色ありはリングの
///   み(中は透明)」という設計どおり、中身は単に何も描かない(背景を
///   `bg()` しない)ので、行/チップ自身の背景が透けて見える。
/// - 呼吸(M2)は中の塗り(`status_dot_element`)にだけ掛かる -- 輪は
///   `.border_color`の静的な style で、`with_animation` の対象外(呼び出し
///   側が追加のアニメーションを組む必要はない)。
///
/// `anim` は `Option` (第16波follow-up の実バグ修正): 一度も選択されて
/// いない Task は `TaskWorkspace` 自体が無く、`app::LaboLaboApp::
/// task_dot_anim` は `None` を返す -- これまではその場合 `dot_el` ごと
/// `None` になり、カスタム色の輪も行内の幅確保も消えて他行とタイトルの
/// 開始位置がガタつく実バグがあった。`anim` が `None` の間は
/// [`status_dot_element`]のクロスフェード/呼吸を使わず
/// [`static_dot_fill`]の単発描画にフォールバックする(状態が変わった
/// 瞬間の色遷移アニメーションは、そもそもまだロードされていない
/// Task には無縁なので、フォールバックしても実害は無い)ことで、
/// カスタム色の輪だけは(状態が無くても)従来どおり描ける。行内の幅
/// 確保自体は呼び出し側(`sidebar.rs`/`task_workspace.rs`)が
/// [`DOT_RING_SIZE`]固定の枠を**無条件に**確保する側の責務 -- この関数の
/// 戻り値が `None` でも枠だけは残る。
pub fn unified_dot_element(
    id_base: impl std::fmt::Display,
    status_color: Option<u32>,
    custom_color: Option<u32>,
    is_running: bool,
    breathing_enabled: bool,
    anim: Option<&Cell<DotAnimState>>,
) -> Option<AnyElement> {
    let params = unified_dot_params(status_color, custom_color);
    let fill_el = match anim {
        Some(anim) => status_dot_element(
            id_base,
            params.fill_target,
            is_running,
            breathing_enabled,
            anim,
        ),
        None => static_dot_fill(params.fill_target),
    };
    let Some(ring_color) = params.ring_color else {
        return fill_el;
    };
    Some(
        div()
            .w(px(DOT_RING_SIZE))
            .h(px(DOT_RING_SIZE))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .border(px(DOT_RING_WIDTH))
            .border_color(rgb(ring_color))
            .children(fill_el)
            .into_any_element(),
    )
}

/// A plain, unanimated [`STATUS_DOT_SIZE`] fill dot at `target`'s color --
/// [`unified_dot_element`]'s fallback when there's no live [`DotAnimState`]
/// to crossfade/breathe against (see that function's doc comment for when
/// this is reached). `None` (no element at all, same as
/// [`status_dot_element`]'s own "nothing observed yet" case) when `target`
/// itself is `None`.
fn static_dot_fill(target: Option<u32>) -> Option<AnyElement> {
    target.map(|color| {
        div()
            .w(px(STATUS_DOT_SIZE))
            .h(px(STATUS_DOT_SIZE))
            .rounded_full()
            .bg(rgb(color))
            .into_any_element()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - cubic_bezier

    #[test]
    fn cubic_bezier_hits_both_endpoints() {
        let ease = ease_standard();
        assert!((ease(0.0) - 0.0).abs() < 1e-4);
        assert!((ease(1.0) - 1.0).abs() < 1e-4);
        let strong = ease_out_strong();
        assert!((strong(0.0) - 0.0).abs() < 1e-4);
        assert!((strong(1.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn cubic_bezier_linear_control_points_are_identity() {
        // cubic-bezier(x, x, y, y) with x1==y1 and x2==y2 collapses to the
        // straight diagonal -- the simplest sanity check that the
        // Newton-Raphson solve is actually inverting x(t) correctly.
        let linear = cubic_bezier(0.3, 0.3, 0.7, 0.7);
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            assert!((linear(x) - x).abs() < 1e-3, "x={x} => {}", linear(x));
        }
    }

    #[test]
    fn cubic_bezier_stays_within_unit_range() {
        // gpui's `AnimationElement` debug_asserts the eased delta is in
        // 0.0..=1.0 -- both curves this module ships must never overshoot.
        // Checked via two boxed closures (rather than an array literal)
        // since `ease_standard()`/`ease_out_strong()` return distinct
        // opaque `impl Fn` types that can't share one array's element type.
        let eases: [Box<dyn Fn(f32) -> f32>; 2] =
            [Box::new(ease_standard()), Box::new(ease_out_strong())];
        for ease in eases {
            for i in 0..=20 {
                let x = i as f32 / 20.0;
                let y = ease(x);
                assert!((0.0..=1.0).contains(&y), "x={x} => y={y}");
            }
        }
    }

    #[test]
    fn ease_standard_is_monotonically_increasing() {
        let ease = ease_standard();
        let mut previous = ease(0.0);
        for i in 1..=20 {
            let y = ease(i as f32 / 20.0);
            assert!(y >= previous - 1e-6, "eased curve must not go backwards");
            previous = y;
        }
    }

    // MARK: - breath_opacity

    #[test]
    fn breath_opacity_peaks_at_cycle_boundaries() {
        assert!((breath_opacity(0.0) - 1.0).abs() < 1e-4);
        assert!((breath_opacity(1.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn breath_opacity_troughs_at_the_midpoint() {
        assert!((breath_opacity(0.5) - 0.55).abs() < 1e-4);
    }

    #[test]
    fn breath_opacity_stays_within_the_configured_range() {
        for i in 0..=100 {
            let t = i as f32 / 100.0;
            let opacity = breath_opacity(t);
            assert!((0.55..=1.0).contains(&opacity), "t={t} => {opacity}");
        }
    }

    #[test]
    fn breath_opacity_wraps_repeating_input() {
        // `Animation::repeat()` hands the easing fn a delta that wraps via
        // `%= 1.0`, but a stray >1.0 sample should still behave sanely.
        assert!((breath_opacity(1.25) - breath_opacity(0.25)).abs() < 1e-4);
    }

    // MARK: - lerp_dot_color

    #[test]
    fn lerp_dot_color_at_zero_is_the_from_color() {
        assert_eq!(
            lerp_dot_color(Some(0x30d158), Some(0xffd60a), 0.0),
            0x30d158ff
        );
    }

    #[test]
    fn lerp_dot_color_at_one_is_the_to_color() {
        assert_eq!(
            lerp_dot_color(Some(0x30d158), Some(0xffd60a), 1.0),
            0xffd60aff
        );
    }

    #[test]
    fn lerp_dot_color_at_half_averages_each_channel() {
        // 0x30d158 -> (0x30, 0xd1, 0x58) = (48, 209, 88)
        // 0xffd60a -> (0xff, 0xd6, 0x0a) = (255, 214, 10)
        // midpoints: (151.5, 211.5, 49) -> rounds to (152, 212, 49) = 0x98d431
        assert_eq!(
            lerp_dot_color(Some(0x30d158), Some(0xffd60a), 0.5),
            0x98d431ff
        );
    }

    #[test]
    fn lerp_dot_color_fades_in_from_no_dot() {
        // None -> Some: starts fully transparent at the target's hue, ends
        // fully opaque.
        assert_eq!(lerp_dot_color(None, Some(0x30d158), 0.0), 0x30d15800);
        assert_eq!(lerp_dot_color(None, Some(0x30d158), 1.0), 0x30d158ff);
    }

    #[test]
    fn lerp_dot_color_fades_out_to_no_dot() {
        // Some -> None: starts fully opaque at the source's hue, ends fully
        // transparent (still the source's hue, so it visibly fades rather
        // than jumping to black).
        assert_eq!(lerp_dot_color(Some(0x30d158), None, 0.0), 0x30d158ff);
        assert_eq!(lerp_dot_color(Some(0x30d158), None, 1.0), 0x30d15800);
    }

    #[test]
    fn lerp_dot_color_clamps_out_of_range_t() {
        assert_eq!(
            lerp_dot_color(Some(0x000000), Some(0xffffff), -1.0),
            lerp_dot_color(Some(0x000000), Some(0xffffff), 0.0)
        );
        assert_eq!(
            lerp_dot_color(Some(0x000000), Some(0xffffff), 2.0),
            lerp_dot_color(Some(0x000000), Some(0xffffff), 1.0)
        );
    }

    // MARK: - advance_dot

    #[test]
    fn advance_dot_records_no_transition_when_unchanged() {
        let anim = Cell::new(DotAnimState::default());
        let first = advance_dot(&anim, Some(0x30d158));
        let second = advance_dot(&anim, Some(0x30d158));
        assert_eq!(first.generation, second.generation);
        assert_eq!(second.to, Some(0x30d158));
    }

    #[test]
    fn advance_dot_bumps_generation_on_change() {
        let anim = Cell::new(DotAnimState::default());
        let first = advance_dot(&anim, Some(0x30d158));
        let second = advance_dot(&anim, Some(0xffd60a));
        assert_ne!(first.generation, second.generation);
        assert_eq!(second.from, Some(0x30d158));
        assert_eq!(second.to, Some(0xffd60a));
    }

    #[test]
    fn advance_dot_from_none_target_with_no_history_is_a_no_op() {
        let anim = Cell::new(DotAnimState::default());
        let frame = advance_dot(&anim, None);
        assert_eq!(frame.from, None);
        assert_eq!(frame.to, None);
    }

    #[test]
    fn advance_dot_tracks_a_fade_out_transition() {
        let anim = Cell::new(DotAnimState::default());
        advance_dot(&anim, Some(0x30d158));
        let faded = advance_dot(&anim, None);
        assert_eq!(faded.from, Some(0x30d158));
        assert_eq!(faded.to, None);
    }

    // MARK: - unified_dot_params (第16波 #2)

    #[test]
    fn unified_dot_params_ring_always_tracks_custom_color_regardless_of_status() {
        assert_eq!(
            unified_dot_params(None, Some(0x30d158)).ring_color,
            Some(0x30d158)
        );
        assert_eq!(
            unified_dot_params(Some(0xffd60a), Some(0x30d158)).ring_color,
            Some(0x30d158)
        );
    }

    #[test]
    fn unified_dot_params_no_custom_color_means_no_ring() {
        assert_eq!(unified_dot_params(Some(0xffd60a), None).ring_color, None);
        assert_eq!(unified_dot_params(None, None).ring_color, None);
    }

    #[test]
    fn unified_dot_params_fill_always_tracks_status_regardless_of_custom_color() {
        assert_eq!(
            unified_dot_params(Some(0xffd60a), Some(0x30d158)).fill_target,
            Some(0xffd60a)
        );
        assert_eq!(unified_dot_params(None, Some(0x30d158)).fill_target, None);
    }

    // MARK: - reduce_motion

    #[test]
    fn reduce_motion_is_false_when_unset() {
        assert!(!reduce_motion_from(None));
    }

    #[test]
    fn reduce_motion_is_true_only_for_the_literal_string_one() {
        assert!(reduce_motion_from(Some("1".to_string())));
        assert!(!reduce_motion_from(Some("0".to_string())));
        assert!(!reduce_motion_from(Some("true".to_string())));
        assert!(!reduce_motion_from(Some("".to_string())));
    }
}
