//! ウィンドウ位置・サイズの記憶 (wave 6c §3)。
//!
//! `LaboLaboApp::new` の `observe_window_bounds` が移動/リサイズを検知し、
//! ~500ms デバウンスして `TaskDatabase` の `appState` (`windowBounds` キー)
//! へ JSON `{"x":..,"y":..,"w":..,"h":..}` で保存する（`app.rs` の
//! `schedule_window_bounds_save`）。起動時は `main.rs` が保存値を読み、
//! **現在接続中のどのディスプレイとも交差しない場合は中央配置へフォール
//! バック**する（外部モニタを外した後の「画面外に復元されて掴めない」事故
//! の防止 -- [`restore_bounds`]）。
//!
//! フルスクリーン/最大化は初版では通常ウィンドウとして復元する（保存側が
//! `WindowBounds::get_bounds()` の restore サイズを保存し、復元側は常に
//! `WindowBounds::Windowed` で開く）-- crate README に明記。
//!
//! この module は encode/decode/交差判定の純ロジックのみ（gpui の
//! `Bounds<Pixels>` はプレーンなデータ型なので、`App`/`Window` なしで
//! ユニットテストできる）。DB 読み書き・デバウンスは呼び出し側の仕事。

use gpui::{point, px, size, Bounds, Pixels};

/// 保存形式（JSON の中間表現）。gpui のグローバル座標系（マルチディスプレイ
/// 横断）の論理ピクセル。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SavedWindowBounds {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl SavedWindowBounds {
    pub fn from_bounds(bounds: Bounds<Pixels>) -> Self {
        Self {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            w: f32::from(bounds.size.width),
            h: f32::from(bounds.size.height),
        }
    }

    pub fn to_bounds(self) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(px(self.w), px(self.h)),
        }
    }
}

/// `{"x":..,"y":..,"w":..,"h":..}` へエンコード（`TaskDatabase::
/// set_window_bounds` へ渡す文字列）。
pub fn encode(saved: SavedWindowBounds) -> String {
    serde_json::json!({
        "x": saved.x,
        "y": saved.y,
        "w": saved.w,
        "h": saved.h,
    })
    .to_string()
}

/// [`encode`] の逆。欠けたキー・数値でない値・壊れた JSON・非有限値・
/// 正でない幅/高さは `None`（「保存なし」と同じ扱い = 中央フォールバック）。
pub fn decode(json: &str) -> Option<SavedWindowBounds> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let field = |key: &str| value.get(key).and_then(|v| v.as_f64()).map(|v| v as f32);
    let saved = SavedWindowBounds {
        x: field("x")?,
        y: field("y")?,
        w: field("w")?,
        h: field("h")?,
    };
    let all_finite = [saved.x, saved.y, saved.w, saved.h]
        .iter()
        .all(|v| v.is_finite());
    if !all_finite || saved.w <= 0.0 || saved.h <= 0.0 {
        return None;
    }
    Some(saved)
}

/// 保存された bounds が `displays`（各接続ディスプレイの bounds）のいずれか
/// と交差するならその bounds を、どれとも交差しない（画面外に取り残される）
/// なら `None` を返す -- `None` は呼び出し側の中央配置フォールバック。
/// ディスプレイが 1 枚も報告されない環境（ヘッドレス等）も安全側に倒して
/// `None`。
pub fn restore_bounds(
    saved: SavedWindowBounds,
    displays: &[Bounds<Pixels>],
) -> Option<Bounds<Pixels>> {
    let bounds = saved.to_bounds();
    displays
        .iter()
        .any(|display| display.intersects(&bounds))
        .then_some(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        SavedWindowBounds { x, y, w, h }.to_bounds()
    }

    #[test]
    fn encode_decode_round_trips() {
        let saved = SavedWindowBounds {
            x: 12.5,
            y: -8.0,
            w: 1120.0,
            h: 600.0,
        };
        assert_eq!(decode(&encode(saved)), Some(saved));
    }

    #[test]
    fn decode_rejects_malformed_input() {
        assert_eq!(decode("not json"), None);
        assert_eq!(decode("{}"), None);
        assert_eq!(decode(r#"{"x":1,"y":2,"w":3}"#), None); // h missing
        assert_eq!(decode(r#"{"x":"a","y":2,"w":3,"h":4}"#), None);
        // 幅/高さゼロ・負は「復元不能」として保存なし扱い。
        assert_eq!(decode(r#"{"x":0,"y":0,"w":0,"h":600}"#), None);
        assert_eq!(decode(r#"{"x":0,"y":0,"w":800,"h":-1}"#), None);
    }

    #[test]
    fn restore_keeps_bounds_intersecting_a_display() {
        let displays = [bounds(0.0, 0.0, 1920.0, 1080.0)];
        let saved = SavedWindowBounds {
            x: 100.0,
            y: 100.0,
            w: 800.0,
            h: 600.0,
        };
        assert_eq!(restore_bounds(saved, &displays), Some(saved.to_bounds()));
    }

    #[test]
    fn restore_accepts_partial_overlap() {
        // 画面の右下に一部だけはみ出しているウィンドウは掴めるので復元する。
        let displays = [bounds(0.0, 0.0, 1920.0, 1080.0)];
        let saved = SavedWindowBounds {
            x: 1800.0,
            y: 1000.0,
            w: 800.0,
            h: 600.0,
        };
        assert!(restore_bounds(saved, &displays).is_some());
    }

    #[test]
    fn restore_falls_back_when_off_every_display() {
        // 外したセカンダリモニタ（x>=1920 側）にだけ載っていたウィンドウ。
        let displays = [bounds(0.0, 0.0, 1920.0, 1080.0)];
        let saved = SavedWindowBounds {
            x: 2000.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        assert_eq!(restore_bounds(saved, &displays), None);
    }

    #[test]
    fn restore_checks_all_displays() {
        let displays = [
            bounds(0.0, 0.0, 1920.0, 1080.0),
            bounds(1920.0, 0.0, 2560.0, 1440.0),
        ];
        let saved = SavedWindowBounds {
            x: 2200.0,
            y: 100.0,
            w: 800.0,
            h: 600.0,
        };
        assert!(restore_bounds(saved, &displays).is_some());
    }

    #[test]
    fn restore_with_no_displays_falls_back() {
        let saved = SavedWindowBounds {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        assert_eq!(restore_bounds(saved, &[]), None);
    }
}
